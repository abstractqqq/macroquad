use super::*;
use crate::native::ClipboardError;
use crate::{wl_request, wl_request_constructor};

use core::ffi::{c_char, c_int, c_void, CStr};

const MIME_TEXT_PLAIN_UTF8: u8 = 1 << 0;
const MIME_TEXT_PLAIN: u8 = 1 << 1;
const MIME_UTF8_STRING: u8 = 1 << 2;
const MIME_TEXT: u8 = 1 << 3;
const MIME_STRING: u8 = 1 << 4;

const OFFERED_MIMES: [&CStr; 5] = [
    c"text/plain;charset=utf-8",
    c"text/plain",
    c"UTF8_STRING",
    c"TEXT",
    c"STRING",
];

pub(super) unsafe extern "C" fn data_offer_handle_offer(
    data: *mut c_void,
    data_offer: *mut wl_data_offer,
    mime_type: *const c_char,
) {
    if data.is_null() || data_offer.is_null() || mime_type.is_null() {
        return;
    }
    let display: &mut WaylandPayload = &mut *(data as *mut _);
    if display.clipboard.pending_offer != Some(data_offer) {
        display.clipboard.pending_offer = Some(data_offer);
        display.clipboard.pending_mime_mask = 0;
    }
    let flag = match CStr::from_ptr(mime_type).to_bytes() {
        b"text/plain;charset=utf-8" => MIME_TEXT_PLAIN_UTF8,
        b"text/plain" => MIME_TEXT_PLAIN,
        b"UTF8_STRING" => MIME_UTF8_STRING,
        b"TEXT" => MIME_TEXT,
        b"STRING" => MIME_STRING,
        _ => 0,
    };
    display.clipboard.pending_mime_mask |= flag;
}

// new clipboard content is available
// this could be fired at any time, so we just store the `data_offer` for later use
pub(super) unsafe extern "C" fn data_device_handle_selection(
    data: *mut c_void,
    data_device: *mut wl_data_device,
    data_offer: *mut wl_data_offer,
) {
    let display: &mut WaylandPayload = &mut *(data as *mut _);
    assert_eq!(data_device, display.data_device);
    let new_offer = (!data_offer.is_null()).then_some(data_offer);
    if let Some(old_offer) = display.clipboard.data_offer.take() {
        if Some(old_offer) != new_offer {
            wl_request!(display.client, old_offer, WL_DATA_OFFER_DESTROY);
            (display.client.wl_proxy_destroy)(old_offer as _);
        }
    }
    display.clipboard.data_offer = new_offer;
    display.clipboard.offer_mime_mask = if new_offer.is_some()
        && display.clipboard.pending_offer == new_offer
    {
        display.clipboard.pending_mime_mask
    } else {
        0
    };
}

#[derive(Default, Debug)]
pub(super) struct ClipboardContext {
    content: String,
    data_source: Option<*mut wl_data_source>,
    data_offer: Option<*mut wl_data_offer>,
    offer_mime_mask: u8,
    pending_offer: Option<*mut wl_data_offer>,
    pending_mime_mask: u8,
}

impl WaylandPayload {
    unsafe fn clipboard_get(&mut self) -> Option<String> {
        if self.clipboard.data_source.is_some() {
            return Some(self.clipboard.content.clone());
        }
        let data_offer = self.clipboard.data_offer?;
        let mask = self.clipboard.offer_mime_mask;
        let mime: &CStr = if mask & MIME_TEXT_PLAIN_UTF8 != 0 {
            c"text/plain;charset=utf-8"
        } else if mask & MIME_TEXT_PLAIN != 0 {
            c"text/plain"
        } else if mask & MIME_UTF8_STRING != 0 {
            c"UTF8_STRING"
        } else if mask & MIME_TEXT != 0 {
            c"TEXT"
        } else if mask & MIME_STRING != 0 {
            c"STRING"
        } else {
            c"text/plain;charset=utf-8"
        };
        let bytes = self
            .client
            .data_offer_receive(self.display, data_offer, mime.as_ptr())?;
        String::from_utf8(bytes).ok()
    }

    unsafe fn clipboard_set(&mut self, data: &str) -> Result<(), ClipboardError> {
        // Wayland requires that only the window with focus can set the clipboard
        let serial = self
            .keyboard_context
            .enter_serial
            .ok_or(ClipboardError::Unavailable)?;

        self.clipboard.content.clear();
        self.clipboard.content.push_str(data);

        let data_source = self.new_clipboard_data_source()?;
        for mime in OFFERED_MIMES {
            wl_request!(
                self.client,
                data_source,
                WL_DATA_SOURCE_OFFER,
                mime.as_ptr()
            );
        }
        wl_request!(
            self.client,
            self.data_device,
            WL_DATA_DEVICE_SET_SELECTION,
            data_source,
            serial
        );
        (self.client.wl_display_flush)(self.display);
        Ok(())
    }

    unsafe fn respond_to_clipboard_request(&mut self, mime_type: &[u8], fd: c_int) {
        match mime_type {
            b"text/plain;charset=utf-8"
            | b"text/plain"
            | b"UTF8_STRING"
            | b"TEXT"
            | b"STRING" => {
                libc::write(
                    fd,
                    self.clipboard.content.as_ptr() as _,
                    self.clipboard.content.len(),
                );
            }
            _ => {}
        }
        libc::close(fd);
    }

    unsafe fn destroy_clipboard_data_source(&mut self) {
        if let Some(data_source) = self.clipboard.data_source.take() {
            wl_request!(self.client, data_source, WL_DATA_SOURCE_DESTROY);
            (self.client.wl_proxy_destroy)(data_source as _);
        }
    }

    unsafe fn new_clipboard_data_source(&mut self) -> Result<*mut wl_data_source, ClipboardError> {
        self.destroy_clipboard_data_source();
        let data_source: *mut wl_data_source = wl_request_constructor!(
            self.client,
            self.data_device_manager,
            WL_DATA_DEVICE_MANAGER_CREATE_DATA_SOURCE,
            self.client.wl_data_source_interface,
        );
        if data_source.is_null() {
            return Err(ClipboardError::OperationFailed);
        }
        DATA_SOURCE_LISTENER.send = data_source_handle_send;
        DATA_SOURCE_LISTENER.cancelled = data_source_handle_cancelled;
        (self.client.wl_proxy_add_listener)(
            data_source as _,
            &DATA_SOURCE_LISTENER as *const _ as _,
            self as *mut _ as _,
        );
        self.clipboard.data_source = Some(data_source);
        Ok(data_source)
    }
}

static mut DATA_SOURCE_LISTENER: wl_data_source_listener = wl_data_source_listener::dummy();

// some app (could be ourself) is requesting the owned clipboard
unsafe extern "C" fn data_source_handle_send(
    data: *mut c_void,
    data_source: *mut wl_data_source,
    mime_type: *const c_char,
    fd: c_int,
) {
    let display: &mut WaylandPayload = &mut *(data as *mut _);
    if display.clipboard.data_source == Some(data_source) && !mime_type.is_null() {
        let mime_bytes = CStr::from_ptr(mime_type).to_bytes();
        display.respond_to_clipboard_request(mime_bytes, fd);
    } else {
        libc::close(fd);
    }
}

// the owned clipboard has been replaced by some other app
unsafe extern "C" fn data_source_handle_cancelled(
    data: *mut c_void,
    data_source: *mut wl_data_source,
) {
    let display: &mut WaylandPayload = &mut *(data as *mut _);
    if display.clipboard.data_source == Some(data_source) {
        display.destroy_clipboard_data_source();
    }
}

pub struct WaylandClipboard {
    display: *mut WaylandPayload,
}
unsafe impl Send for WaylandClipboard {}
unsafe impl Sync for WaylandClipboard {}

impl WaylandClipboard {
    pub(super) fn new(display: *mut WaylandPayload) -> Self {
        WaylandClipboard { display }
    }

    pub fn get(&mut self) -> Option<String> {
        unsafe { (&mut *self.display).clipboard_get() }
    }

    pub fn set(&mut self, data: &str) -> Result<(), ClipboardError> {
        unsafe { (&mut *self.display).clipboard_set(data) }
    }
}
