//! Copy access rules to an empty staged file before it receives secret contents.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED, UNPROTECTED_DACL_SECURITY_INFORMATION,
};

use crate::error::{NativeError, api_error};

struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: GetNamedSecurityInfoW allocated this descriptor with LocalAlloc.
        // Its borrowed DACL is no longer used and this owner frees it exactly once.
        unsafe { LocalFree(self.0) };
    }
}

pub fn copy_file_access_rules(source: &Path, destination: &Path) -> Result<(), NativeError> {
    let source = wide_path(source)?;
    let destination = wide_path(destination)?;
    let mut descriptor = null_mut();
    let mut dacl = null_mut();
    // SAFETY: Paths are NUL-terminated; output pointers refer to live local storage.
    // Unrequested security fields are null. The returned allocation is owned below.
    let result = unsafe {
        GetNamedSecurityInfoW(
            source.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &raw mut dacl,
            null_mut(),
            &raw mut descriptor,
        )
    };
    check(result, "GetNamedSecurityInfoW")?;
    let descriptor = Descriptor(descriptor);
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: The descriptor remains allocated; outputs are valid local variables.
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &raw mut control, &raw mut revision) }
        == 0
    {
        return Err(api_error("GetSecurityDescriptorControl"));
    }
    let inheritance = if control & SE_DACL_PROTECTED != 0 {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    // SAFETY: The destination path and descriptor-backed DACL remain alive during
    // this call. Only the DACL and its inheritance flag are changed on the staged file.
    let result = unsafe {
        SetNamedSecurityInfoW(
            destination.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | inheritance,
            null_mut(),
            null_mut(),
            dacl,
            null_mut(),
        )
    };
    check(result, "SetNamedSecurityInfoW")
}

fn check(code: u32, operation: &'static str) -> Result<(), NativeError> {
    if code == 0 {
        return Ok(());
    }
    Err(NativeError::Api {
        operation,
        source: std::io::Error::from_raw_os_error(code.cast_signed()),
    })
}

fn wide_path(path: &Path) -> Result<Vec<u16>, NativeError> {
    let mut path: Vec<_> = path.as_os_str().encode_wide().collect();
    if path.contains(&0) {
        return Err(NativeError::InteriorNul("file path"));
    }
    path.push(0);
    Ok(path)
}
