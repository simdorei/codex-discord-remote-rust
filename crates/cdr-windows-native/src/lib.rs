//! Narrow, reviewed Win32 boundary for the Windows-first Rust runtime.

#[cfg(windows)]
pub mod protocol;
#[cfg(windows)]
pub mod resources;

#[cfg(windows)]
mod atomic_file;
#[cfg(windows)]
mod file_identity;
#[cfg(windows)]
mod file_security;
#[cfg(windows)]
mod file_transfer;
#[cfg(windows)]
pub use file_identity::{resolved_file_path, same_resolved_path};
#[cfg(windows)]
pub use file_security::copy_file_access_rules;
#[cfg(windows)]
pub use file_transfer::{FileLeaseTransfer, transfer_file_lease};
#[cfg(windows)]
mod delete_file;
#[cfg(windows)]
pub use delete_file::delete_open_file;
#[cfg(windows)]
mod data_protection;
#[cfg(windows)]
mod error;
#[cfg(windows)]
mod probe;
#[cfg(windows)]
mod process;
#[cfg(windows)]
pub use probe::{ProbeOutput, capture_bounded};
#[cfg(windows)]
mod single_instance;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use atomic_file::atomic_replace;
#[cfg(windows)]
pub use data_protection::{protect_current_user, unprotect_current_user};
#[cfg(windows)]
pub use error::NativeError;
#[cfg(windows)]
pub use process::{
    CapturedWindowProcess, LiveProcessMetadata, WindowProcess, current_process_identity,
    query_live_process_metadata,
};
#[cfg(windows)]
pub use single_instance::SingleInstance;
#[cfg(windows)]
pub use window::{
    DeviceWindowInfo, NativeMouseButton, WindowInfo, WindowRect, activate_window,
    capture_window_png, click_at, drag_at, enumerate_visible_windows, find_window_by_title_suffix,
    inspect_visible_window, inspect_window, normalize_keys, post_close, press_device_keys,
    press_keys, scroll_at, set_clipboard_text, type_device_text, type_text,
};
