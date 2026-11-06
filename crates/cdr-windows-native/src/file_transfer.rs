//! Transfer read leases from an owned probe, including handles from partial failures.
use crate::error::{NativeError, api_error};
use crate::process::handle::OwnedHandle;
use std::fs::File;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_DUP_HANDLE,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

const CAPACITY: usize = 8;
const SIZE: usize = CAPACITY * size_of::<HANDLE>();

struct Mapping {
    _handle: OwnedHandle,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}
impl Mapping {
    fn from_handle(handle: OwnedHandle) -> Result<Self, NativeError> {
        // SAFETY: a valid section handle is retained and SIZE is the entire fixed buffer.
        let view = unsafe { MapViewOfFile(handle.raw(), FILE_MAP_ALL_ACCESS, 0, 0, SIZE) };
        if view.Value.is_null() {
            return Err(api_error("MapViewOfFile(lease transfer)"));
        }
        Ok(Self {
            _handle: handle,
            view,
        })
    }
    fn slot(&self, index: usize) -> *mut HANDLE {
        assert!(index < CAPACITY);
        // SAFETY: views are page aligned; the validated index is inside SIZE.
        unsafe { self.view.Value.cast::<HANDLE>().add(index) }
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this is the one retained view returned by MapViewOfFile.
        unsafe {
            UnmapViewOfFile(self.view);
        }
    }
}

/// Collect after the owned helper stops. The page survives helper failure, so
/// duplicated handles never depend on complete stdout delivery.
pub struct FileLeaseTransfer {
    mapping: Mapping,
}

fn wide(name: &str) -> Result<Vec<u16>, NativeError> {
    if !name.starts_with("Local\\cdr-codex-lease-") || name.contains('\0') {
        return Err(NativeError::InvalidInput(
            "invalid lease transfer name".into(),
        ));
    }
    Ok(name.encode_utf16().chain(Some(0)).collect())
}
impl FileLeaseTransfer {
    /// A writer whose exit is unconfirmed may still publish handles. Keep the
    /// section and all target handles owned by this OS process until it exits;
    /// never read, clear or claim to reclaim those slots concurrently. Supported
    /// startup/admin callers treat this outcome as fatal and do not start again.
    pub fn quarantine_until_process_exit(self) {
        std::mem::forget(self);
    }

    pub fn new(name: &str) -> Result<Self, NativeError> {
        let name = wide(name)?;
        let mapping_size = u32::try_from(SIZE)
            .map_err(|_| NativeError::InvalidInput("lease mapping too large".into()))?;
        // SAFETY: paging-file section, fixed size, NUL-terminated name, default current-user ACL.
        let raw = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                mapping_size,
                name.as_ptr(),
            )
        };
        // SAFETY: read immediately after CreateFileMappingW.
        let existing = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let handle =
            OwnedHandle::new(raw).ok_or_else(|| api_error("CreateFileMappingW(lease transfer)"))?;
        if existing {
            return Err(NativeError::InvalidInput(
                "lease transfer name already exists".into(),
            ));
        }
        Ok(Self {
            mapping: Mapping::from_handle(handle)?,
        })
    }
    #[must_use]
    pub fn take_files(&mut self) -> Vec<File> {
        let mut files = vec![];
        for index in 0..CAPACITY {
            let slot = self.mapping.slot(index);
            // SAFETY: the owned helper ended before collection. Each slot is a
            // handle duplicated into THIS process; zeroing transfers ownership once.
            let raw = unsafe {
                let raw = slot.read_volatile();
                slot.write_volatile(std::ptr::null_mut());
                raw
            };
            if let Some(handle) = OwnedHandle::new_file(raw) {
                files.push(handle.into_file());
            }
        }
        files
    }
}
impl Drop for FileLeaseTransfer {
    fn drop(&mut self) {
        drop(self.take_files());
    }
}

/// Called only by the contained helper with the private parent-created name.
pub fn transfer_file_lease(
    name: &str,
    parent_pid: u32,
    parent_identity: &str,
    index: usize,
    file: &File,
) -> Result<(), NativeError> {
    if index >= CAPACITY {
        return Err(NativeError::InvalidInput("too many file leases".into()));
    }
    let name = wide(name)?;
    // SAFETY: NUL-terminated name; opens only the parent's existing section.
    let raw = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, name.as_ptr()) };
    let handle =
        OwnedHandle::new(raw).ok_or_else(|| api_error("OpenFileMappingW(lease transfer)"))?;
    let mapping = Mapping::from_handle(handle)?;
    // SAFETY: requests only duplication rights for the explicit parent process.
    let raw = unsafe {
        OpenProcess(
            PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            parent_pid,
        )
    };
    let parent = OwnedHandle::new(raw).ok_or_else(|| api_error("OpenProcess(lease parent)"))?;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the same retained parent handle has query rights; outputs are distinct FILETIMEs.
    if unsafe {
        GetProcessTimes(
            parent.raw(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return Err(api_error("GetProcessTimes(lease parent)"));
    }
    if exit.dwLowDateTime != 0
        || exit.dwHighDateTime != 0
        || crate::process::format_process_identity(parent_pid, creation) != parent_identity
    {
        return Err(NativeError::InvalidInput(
            "lease parent identity changed".into(),
        ));
    }
    let slot = mapping.slot(index);
    // SAFETY: the helper is the sole writer; the parent collects after its exit.
    if !unsafe { slot.read_volatile() }.is_null() {
        return Err(NativeError::InvalidInput("lease slot occupied".into()));
    }
    // SAFETY: File stays live. The OS writes the target-process handle DIRECTLY
    // into the shared slot; no post-duplication stdout delivery is needed to reclaim it.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            file.as_raw_handle(),
            parent.raw(),
            slot,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(api_error("DuplicateHandle(file lease)"));
    }
    Ok(())
}
