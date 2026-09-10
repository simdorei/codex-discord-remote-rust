mod capture;
mod clipboard;
mod device_identity;
mod device_input;
mod device_keyboard;
mod identity;
mod input;

pub use capture::capture_window_png;
pub use clipboard::set_clipboard_text;
pub use device_identity::{DeviceWindowInfo, enumerate_visible_windows, inspect_visible_window};
pub use device_input::{NativeMouseButton, click_at, drag_at, post_close, scroll_at};
pub use device_keyboard::{press_device_keys, type_device_text};
pub use identity::{
    WindowInfo, WindowRect, activate_window, find_window_by_title_suffix, inspect_window,
};
pub use input::{normalize_keys, press_keys, type_text};
