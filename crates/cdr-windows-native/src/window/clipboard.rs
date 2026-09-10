use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

use crate::error::{NativeError, api_error};

const CF_UNICODETEXT: u32 = 13;

pub fn set_clipboard_text(text: &str) -> Result<(), NativeError> {
    // SAFETY: a null owner requests clipboard access for this process.
    if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
        return Err(api_error("OpenClipboard"));
    }
    let _clipboard = ClipboardGuard;
    // SAFETY: this process currently owns the open clipboard.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(api_error("EmptyClipboard"));
    }
    let data = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    // SAFETY: allocation size is derived from the live byte buffer.
    let raw = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len()) };
    let mut memory = GlobalMemory::new(raw).ok_or_else(|| api_error("GlobalAlloc"))?;
    // SAFETY: the allocation is live and large enough for `data`.
    let pointer = unsafe { GlobalLock(memory.raw()) };
    if pointer.is_null() {
        return Err(api_error("GlobalLock"));
    }
    // SAFETY: source and destination are valid, non-overlapping buffers of `data.len()` bytes.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), pointer.cast::<u8>(), data.len()) };
    // SAFETY: this balances the successful GlobalLock above.
    let _ = unsafe { GlobalUnlock(memory.raw()) };
    // SAFETY: ownership transfers to the clipboard on a non-null return.
    if unsafe { SetClipboardData(CF_UNICODETEXT, memory.raw()) }.is_null() {
        return Err(api_error("SetClipboardData"));
    }
    memory.release();
    Ok(())
}

struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: the guard exists only after OpenClipboard succeeds.
        let _ = unsafe { CloseClipboard() };
    }
}

struct GlobalMemory(Option<HGLOBAL>);

impl GlobalMemory {
    fn new(raw: HGLOBAL) -> Option<Self> {
        (!raw.is_null()).then_some(Self(Some(raw)))
    }
    fn raw(&self) -> HGLOBAL {
        self.0.unwrap_or(std::ptr::null_mut())
    }
    fn release(&mut self) {
        self.0 = None;
    }
}

impl Drop for GlobalMemory {
    fn drop(&mut self) {
        if let Some(raw) = self.0.take() {
            // SAFETY: this guard owns the allocation until clipboard ownership transfers.
            let _ = unsafe { GlobalFree(raw) };
        }
    }
}
