#![allow(dead_code)]

#[cfg(not(target_os = "android"))]
use std::sync::mpsc;

#[derive(Default)]
pub(crate) struct DroppedFiles {
    pub paths: Vec<std::path::PathBuf>,
    pub bytes: Vec<Vec<u8>>,
}
pub(crate) struct NativeDisplayData {
    pub screen_width: i32,
    pub screen_height: i32,
    pub screen_position: (u32, u32),
    pub dpi_scale: f32,
    pub high_dpi: bool,
    pub quit_requested: bool,
    pub quit_ordered: bool,
    #[cfg(target_os = "android")]
    pub native_requests: Box<dyn Fn(Request) + Send>,
    #[cfg(not(target_os = "android"))]
    pub native_requests: mpsc::Sender<Request>,
    pub clipboard: PlatformClipboard,
    pub dropped_files: DroppedFiles,
    pub blocking_event_loop: bool,

    #[cfg(target_vendor = "apple")]
    pub view: crate::native::apple::frameworks::ObjcId,
    #[cfg(any(target_os = "ios", target_os = "tvos"))]
    pub view_ctrl: crate::native::apple::frameworks::ObjcId,
    #[cfg(target_vendor = "apple")]
    pub gfx_api: crate::conf::AppleGfxApi,
}
#[cfg(target_vendor = "apple")]
unsafe impl Send for NativeDisplayData {}
#[cfg(target_vendor = "apple")]
unsafe impl Sync for NativeDisplayData {}

impl NativeDisplayData {
    pub fn new(
        screen_width: i32,
        screen_height: i32,
        #[cfg(target_os = "android")] native_requests: Box<dyn Fn(Request) + Send>,
        #[cfg(not(target_os = "android"))] native_requests: mpsc::Sender<Request>,
        clipboard: PlatformClipboard,
    ) -> NativeDisplayData {
        NativeDisplayData {
            screen_width,
            screen_height,
            screen_position: (0, 0),
            dpi_scale: 1.,
            high_dpi: false,
            quit_requested: false,
            quit_ordered: false,
            native_requests,
            clipboard,
            dropped_files: Default::default(),
            blocking_event_loop: false,
            #[cfg(target_vendor = "apple")]
            gfx_api: crate::conf::AppleGfxApi::OpenGl,
            #[cfg(target_vendor = "apple")]
            view: std::ptr::null_mut(),
            #[cfg(any(target_os = "ios", target_os = "tvos"))]
            view_ctrl: std::ptr::null_mut(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Request {
    ScheduleUpdate,
    SetCursorGrab(bool),
    ShowMouse(bool),
    SetMouseCursor(crate::CursorIcon),
    SetWindowSize { new_width: u32, new_height: u32 },
    SetWindowPosition { new_x: u32, new_y: u32 },
    SetFullscreen(bool),
    ShowKeyboard(bool),
    SetImePosition { x: i32, y: i32 },
    SetImeEnabled(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardError {
    /// Clipboard is unavailable (e.g., window lacks focus on Wayland, or OS clipboard could not be opened).
    Unavailable,
    /// Failed to allocate OS clipboard memory.
    AllocationFailed,
    /// Input text could not be encoded for the platform clipboard (e.g., interior NUL byte).
    InvalidData,
    /// The OS rejected the clipboard operation.
    OperationFailed,
    /// Clipboard access is not supported on this platform.
    Unsupported,
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardError::Unavailable => write!(f, "clipboard is unavailable"),
            ClipboardError::AllocationFailed => write!(f, "clipboard memory allocation failed"),
            ClipboardError::InvalidData => write!(f, "invalid clipboard data"),
            ClipboardError::OperationFailed => write!(f, "clipboard operation failed"),
            ClipboardError::Unsupported => write!(f, "clipboard is not supported on this platform"),
        }
    }
}

impl std::error::Error for ClipboardError {}

#[cfg(target_os = "linux")]
pub enum PlatformClipboard {
    X11(linux_x11::clipboard::X11Clipboard),
    Wayland(linux_wayland::clipboard::WaylandClipboard),
}

#[cfg(target_os = "linux")]
impl PlatformClipboard {
    pub fn get(&mut self) -> Option<String> {
        match self {
            PlatformClipboard::X11(c) => c.get(),
            PlatformClipboard::Wayland(c) => c.get(),
        }
    }

    pub fn set(&mut self, data: &str) -> Result<(), ClipboardError> {
        match self {
            PlatformClipboard::X11(c) => c.set(data),
            PlatformClipboard::Wayland(c) => c.set(data),
        }
    }
}

#[cfg(target_os = "windows")]
pub type PlatformClipboard = windows::clipboard::WindowsClipboard;

#[cfg(target_os = "macos")]
pub type PlatformClipboard = macos::MacosClipboard;

#[cfg(any(target_os = "ios", target_os = "tvos"))]
pub type PlatformClipboard = ios::IosClipboard;

#[cfg(target_os = "android")]
pub type PlatformClipboard = android::AndroidClipboard;

#[cfg(target_arch = "wasm32")]
pub type PlatformClipboard = wasm::WasmClipboard;

pub mod module;

#[cfg(target_os = "linux")]
pub mod linux_x11;

#[cfg(target_os = "linux")]
pub mod linux_wayland;

#[cfg(target_os = "android")]
pub mod android;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "android")]
pub use android::*;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos"))]
pub mod apple;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(any(target_os = "ios", target_os = "tvos"))]
pub mod ios;

#[cfg(any(target_os = "android", target_os = "linux"))]
pub mod egl;

// there is no glGetProcAddr on webgl, so its impossible to make "gl" module work
// on macos.. well, there is, but way easier to just statically link to gl
#[cfg(not(target_arch = "wasm32"))]
pub mod gl;

#[cfg(target_arch = "wasm32")]
pub use wasm::webgl as gl;

pub mod query_stab;
