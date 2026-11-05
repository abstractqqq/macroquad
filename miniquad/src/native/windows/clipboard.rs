use winapi::um::winbase::{GlobalAlloc, GlobalFree, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use winapi::um::winuser::CF_UNICODETEXT;
use winapi::um::winuser::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};

use crate::native::ClipboardError;
use std::ptr;

struct ClipboardGuard;
impl ClipboardGuard {
    unsafe fn open() -> Option<Self> {
        let result = OpenClipboard(ptr::null_mut());
        if result == false as _ {
            return None;
        }
        Some(ClipboardGuard)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

unsafe fn get_raw_clipboard() -> Option<String> {
    // https://docs.microsoft.com/en-us/windows/win32/dataxchg/about-the-clipboard
    let _guard = ClipboardGuard::open()?;

    // Returns a handle to a clipboard object
    let clipboard_data = GetClipboardData(CF_UNICODETEXT);
    if clipboard_data.is_null() {
        return None;
    }

    let data_ptr = GlobalLock(clipboard_data) as *const u16;
    if data_ptr.is_null() {
        return None;
    }
    let data_len = GlobalSize(clipboard_data) as usize / std::mem::size_of::<u16>();

    let slice = std::slice::from_raw_parts(data_ptr, data_len);
    let len = slice.iter().position(|b| *b == 0).unwrap_or(data_len);
    let result = String::from_utf16_lossy(&slice[..len]);

    GlobalUnlock(clipboard_data);

    Some(result)
}

unsafe fn set_raw_clipboard(data: *const u8, len: usize) -> Result<(), ClipboardError> {
    let _guard = ClipboardGuard::open().ok_or(ClipboardError::Unavailable)?;

    if EmptyClipboard() == 0 {
        return Err(ClipboardError::OperationFailed);
    }

    let alloc_handle = GlobalAlloc(GMEM_MOVEABLE, len);
    if alloc_handle.is_null() {
        return Err(ClipboardError::AllocationFailed);
    }

    let lock = GlobalLock(alloc_handle) as *mut u8;
    if lock.is_null() {
        GlobalFree(alloc_handle);
        return Err(ClipboardError::OperationFailed);
    }
    ptr::copy_nonoverlapping(data, lock, len);
    GlobalUnlock(alloc_handle);

    if SetClipboardData(CF_UNICODETEXT, alloc_handle).is_null() {
        GlobalFree(alloc_handle);
        return Err(ClipboardError::OperationFailed);
    }

    Ok(())
}

pub struct WindowsClipboard {}
impl WindowsClipboard {
    pub fn new() -> WindowsClipboard {
        WindowsClipboard {}
    }

    pub fn get(&mut self) -> Option<String> {
        unsafe { get_raw_clipboard() }
    }

    pub fn set(&mut self, data: &str) -> Result<(), ClipboardError> {
        unsafe {
            let text_w: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
            set_raw_clipboard(
                text_w.as_ptr() as *const u8,
                text_w.len() * std::mem::size_of::<u16>(),
            )
        }
    }
}
