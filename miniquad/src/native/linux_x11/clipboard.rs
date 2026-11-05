#![allow(non_upper_case_globals, non_snake_case)]

//! Clipboard implementation for X11
//!
//! Clipboard API on X11 is pretty weird
//! <https://www.uninformativ.de/blog/postings/2017-04-02/0/POSTING-en.html>
//! so use this with caution.

use super::libx11::*;
use crate::native::ClipboardError;
use std::time::{Duration, Instant};

const CLIPBOARD_TIMEOUT: Duration = Duration::from_millis(250);
const POLL_SLEEP: Duration = Duration::from_micros(200);

unsafe fn get_clipboard(
    libx11: &mut LibX11,
    display: *mut Display,
    window: Window,
    format: Atom,
    owned_message: Option<&str>,
) -> Option<Vec<u8>> {
    // Ask X server to ask current clipboard owner to convert selection to the desired format.
    (libx11.XConvertSelection)(
        display,
        libx11.extensions.clipboard,
        format,
        libx11.extensions.xsel_data,
        window,
        CurrentTime as Time,
    );
    (libx11.XFlush)(display);

    // Wait specifically for SelectionNotify/SelectionRequest on our window without
    // consuming unrelated input or window events from the X11 queue, and bound
    // the wait with a timeout so an unresponsive owner cannot hang the app.
    let deadline = Instant::now() + CLIPBOARD_TIMEOUT;
    let mut event = XEvent { type_0: 0 };
    loop {
        if (libx11.XCheckTypedWindowEvent)(display, window, SelectionNotify, &mut event) != 0 {
            if event.xselection.selection == libx11.extensions.clipboard {
                return (event.xselection.property == libx11.extensions.xsel_data).then(|| {
                    get_property_bytes(libx11, display, window, event.xselection.property)
                });
            }
        }
        if (libx11.XCheckTypedWindowEvent)(display, window, SelectionRequest, &mut event) != 0 {
            respond_to_clipboard_request(libx11, display, &event, owned_message);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL_SLEEP);
    }
}

/// Get the bytes from the window property
/// INCR protocol is implemented; will block until all data are received
/// Useful for getting data following a SelectionNotify event
//
//  X11 has the INCR protocol to transfer data in several batches
//  https://www.x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.html#INCR_Properties
//
//  The data are sequentially written to the given property on the window
//  After each read, we delete the property to notify X that we are ready to receive more data
//  Used both for the clipboard and the drag-n-drop protocol
pub(crate) unsafe fn get_property_bytes(
    libx11: &mut LibX11,
    display: *mut Display,
    window: Window,
    property: Atom,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buf = std::ptr::null_mut::<libc::c_char>();
    let mut size: libc::c_ulong = 0;
    let mut actual_type = 0 as Atom;
    let mut actual_format: libc::c_int = 0;
    let mut _size_after: libc::c_ulong = 0;
    let mut incr_mode = false;
    loop {
        (libx11.XGetWindowProperty)(
            display,
            window,
            property,
            0 as _,
            libc::c_long::MAX,
            false as _,
            AnyPropertyType,
            &mut actual_type,
            &mut actual_format,
            &mut size,
            &mut _size_after,
            &mut buf as *mut *mut libc::c_char as _,
        );

        if actual_type == libx11.extensions.incr {
            // we are in INCR mode
            incr_mode = true;
            if !buf.is_null() {
                (libx11.XFree)(buf as *mut libc::c_void);
                buf = std::ptr::null_mut();
            }
        } else if size == 0 {
            // no more data to read
            if !buf.is_null() {
                (libx11.XFree)(buf as *mut libc::c_void);
            }
            return bytes;
        } else {
            let n_bits = match actual_format {
                8 => std::mem::size_of::<libc::c_char>(),
                16 => std::mem::size_of::<libc::c_int>(),
                32 => std::mem::size_of::<libc::c_long>(),
                _ => unreachable!(),
            };
            bytes.extend(std::slice::from_raw_parts(
                buf as *const _,
                n_bits * size as usize,
            ));
            (libx11.XFree)(buf as *mut libc::c_void);
            buf = std::ptr::null_mut();

            // if not in INCR mode, then we've got all the data
            if !incr_mode {
                return bytes;
            }
        }

        if incr_mode {
            (libx11.XDeleteProperty)(display, window, property);
            (libx11.XFlush)(display);
            // wait until we get a new batch without discarding unrelated events
            let deadline = Instant::now() + CLIPBOARD_TIMEOUT;
            let mut event = XEvent { type_0: 0 };
            loop {
                if (libx11.XCheckTypedWindowEvent)(display, window, PropertyNotify, &mut event) != 0
                {
                    if event.xproperty.atom == property && event.xproperty.state == PropertyNewValue
                    {
                        break;
                    }
                }
                if Instant::now() >= deadline {
                    return bytes;
                }
                std::thread::sleep(POLL_SLEEP);
            }
        }
    }
}

/// Respond to an X11 SelectionRequest event when our window owns the clipboard.
pub(crate) unsafe fn respond_to_clipboard_request(
    libx11: &mut LibX11,
    display: *mut Display,
    event: *const XEvent,
    owned_message: Option<&str>,
) {
    assert!((*event).type_0 == SelectionRequest);

    let message = owned_message.unwrap_or("");
    let xselectionrequest = (*event).xselectionrequest;
    let property = if xselectionrequest.property != 0 {
        xselectionrequest.property
    } else {
        xselectionrequest.target
    };
    let mut ev = XSelectionEvent {
        type_0: SelectionNotify,
        serial: 0,
        send_event: 0,
        display: xselectionrequest.display,
        requestor: xselectionrequest.requestor,
        selection: xselectionrequest.selection,
        target: xselectionrequest.target,
        property,
        time: xselectionrequest.time,
    };

    let exts = &libx11.extensions;
    let target = xselectionrequest.target;

    if target == exts.targets {
        let supported_targets: [Atom; 6] = [
            exts.targets,
            exts.utf8_string,
            exts.mime_text_plain_utf8,
            exts.mime_text_plain,
            exts.text,
            XA_STRING,
        ];
        (libx11.XChangeProperty)(
            xselectionrequest.display,
            xselectionrequest.requestor,
            property,
            XA_ATOM,
            32 as libc::c_int,
            PropModeReplace,
            supported_targets.as_ptr() as *const libc::c_uchar,
            supported_targets.len() as _,
        );
    } else if target == exts.utf8_string
        || target == exts.mime_text_plain_utf8
        || target == exts.mime_text_plain
        || target == exts.text
        || target == XA_STRING
    {
        (libx11.XChangeProperty)(
            xselectionrequest.display,
            xselectionrequest.requestor,
            property,
            target,
            8 as libc::c_int,
            PropModeReplace,
            message.as_bytes().as_ptr(),
            message.len() as _,
        );
    } else {
        // signal X that request is denied
        ev.property = 0 as Atom;
    }

    (libx11.XSendEvent)(
        display,
        ev.requestor,
        false as _,
        NoEventMask,
        &mut ev as *mut XSelectionEvent as *mut XEvent,
    );
    (libx11.XFlush)(display);
}

pub struct X11Clipboard {
    libx11: LibX11,
    display: *mut Display,
    window: Window,
    message: Option<String>,
}
unsafe impl Send for X11Clipboard {}
unsafe impl Sync for X11Clipboard {}

impl X11Clipboard {
    pub fn new(libx11: LibX11, display: *mut Display, window: Window) -> X11Clipboard {
        X11Clipboard {
            libx11,
            display,
            window,
            message: None,
        }
    }

    pub(crate) fn clear_ownership(&mut self) {
        self.message = None;
    }

    pub(crate) unsafe fn respond_to_request(&mut self, event: *const XEvent) {
        let msg = self.message.as_deref();
        respond_to_clipboard_request(&mut self.libx11, self.display, event, msg);
    }

    pub fn get(&mut self) -> Option<String> {
        unsafe {
            let owner =
                (self.libx11.XGetSelectionOwner)(self.display, self.libx11.extensions.clipboard);
            if owner == 0 {
                return None;
            }
            if owner == self.window {
                return self.message.clone();
            }

            let utf8_string = self.libx11.extensions.utf8_string;
            let msg_ref = self.message.as_deref();
            let bytes = get_clipboard(
                &mut self.libx11,
                self.display,
                self.window,
                utf8_string,
                msg_ref,
            )
            .or_else(|| {
                get_clipboard(
                    &mut self.libx11,
                    self.display,
                    self.window,
                    XA_STRING,
                    msg_ref,
                )
            })?;
            String::from_utf8(bytes).ok()
        }
    }

    pub fn set(&mut self, data: &str) -> Result<(), ClipboardError> {
        unsafe {
            if self.display.is_null() || self.window == 0 {
                return Err(ClipboardError::Unavailable);
            }
            (self.libx11.XSetSelectionOwner)(
                self.display,
                self.libx11.extensions.clipboard,
                self.window,
                CurrentTime as Time,
            );
            (self.libx11.XFlush)(self.display);
            if (self.libx11.XGetSelectionOwner)(self.display, self.libx11.extensions.clipboard)
                != self.window
            {
                return Err(ClipboardError::OperationFailed);
            }
            self.message = Some(data.to_owned());
            Ok(())
        }
    }
}
