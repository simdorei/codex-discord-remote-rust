use std::ffi::OsStr;
use std::mem::size_of;

use windows_sys::Win32::Foundation::{
    GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    UpdateProcThreadAttribute,
};

use super::encoding::wide;
use super::handle::OwnedHandle;
use crate::error::{NativeError, api_error};

pub struct CapturedStdio {
    attributes: AttributeList,
    inherited: Vec<HANDLE>,
    stdout_read: Option<OwnedHandle>,
    stdout_write: OwnedHandle,
    stderr_read: Option<OwnedHandle>,
    stderr_write: OwnedHandle,
    stdin_read: OwnedHandle,
    stdin_write: Option<OwnedHandle>,
}

impl CapturedStdio {
    pub fn new() -> Result<Self, NativeError> {
        let (stdout_read, stdout_write) = output_pipe()?;
        let (stderr_read, stderr_write) = output_pipe()?;
        let stdin_read = null_input()?;
        Self::build(
            stdout_read,
            stdout_write,
            stderr_read,
            stderr_write,
            stdin_read,
            None,
        )
    }

    pub fn new_piped() -> Result<Self, NativeError> {
        let (stdout_read, stdout_write) = output_pipe()?;
        let (stderr_read, stderr_write) = output_pipe()?;
        let (stdin_read, stdin_write) = input_pipe()?;
        Self::build(
            stdout_read,
            stdout_write,
            stderr_read,
            stderr_write,
            stdin_read,
            Some(stdin_write),
        )
    }

    fn build(
        stdout_read: OwnedHandle,
        stdout_write: OwnedHandle,
        stderr_read: OwnedHandle,
        stderr_write: OwnedHandle,
        stdin_read: OwnedHandle,
        stdin_write: Option<OwnedHandle>,
    ) -> Result<Self, NativeError> {
        let inherited = vec![stdin_read.raw(), stdout_write.raw(), stderr_write.raw()];
        let attributes = AttributeList::for_handles(&inherited)?;
        Ok(Self {
            attributes,
            inherited,
            stdout_read: Some(stdout_read),
            stdout_write,
            stderr_read: Some(stderr_read),
            stderr_write,
            stdin_read,
            stdin_write,
        })
    }

    pub fn startup_info(&self) -> STARTUPINFOEXW {
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb =
            u32::try_from(size_of::<STARTUPINFOEXW>()).expect("STARTUPINFOEXW size");
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = self.stdin_read.raw();
        startup.StartupInfo.hStdOutput = self.stdout_write.raw();
        startup.StartupInfo.hStdError = self.stderr_write.raw();
        startup.lpAttributeList = self.attributes.raw();
        startup
    }

    pub fn into_parent_files(mut self) -> (std::fs::File, std::fs::File) {
        let stdout = self
            .stdout_read
            .take()
            .expect("captured stdout handle")
            .into_file();
        let stderr = self
            .stderr_read
            .take()
            .expect("captured stderr handle")
            .into_file();
        (stdout, stderr)
    }

    pub fn into_piped_parent_files(mut self) -> (std::fs::File, std::fs::File, std::fs::File) {
        let stdin = self
            .stdin_write
            .take()
            .expect("captured stdin handle")
            .into_file();
        let (stdout, stderr) = self.into_parent_files();
        (stdin, stdout, stderr)
    }

    pub fn inherited_count(&self) -> usize {
        self.inherited.len()
    }
}

fn raw_pipe() -> Result<(OwnedHandle, OwnedHandle), NativeError> {
    let attributes = inheritable_security_attributes();
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    // SAFETY: output pointers and the initialized security attributes remain valid for this call.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, &raw const attributes, 0) } == 0 {
        return Err(api_error("CreatePipe"));
    }
    let handles = (OwnedHandle::new(read), OwnedHandle::new(write));
    let (Some(read), Some(write)) = handles else {
        return Err(NativeError::InvalidInput(
            "CreatePipe returned an invalid handle".into(),
        ));
    };
    Ok((read, write))
}

fn output_pipe() -> Result<(OwnedHandle, OwnedHandle), NativeError> {
    let (read, write) = raw_pipe()?;
    // SAFETY: `read` is valid; only the child-facing write handle remains inherited.
    if unsafe { SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(api_error("SetHandleInformation"));
    }
    Ok((read, write))
}

fn input_pipe() -> Result<(OwnedHandle, OwnedHandle), NativeError> {
    let (read, write) = raw_pipe()?;
    // SAFETY: `write` is valid; only the child-facing read handle remains inherited.
    if unsafe { SetHandleInformation(write.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(api_error("SetHandleInformation"));
    }
    Ok((read, write))
}

fn null_input() -> Result<OwnedHandle, NativeError> {
    let name = wide(OsStr::new("NUL"), "null input")?;
    let attributes = inheritable_security_attributes();
    // SAFETY: the NUL name and security attributes remain valid for this call.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &raw const attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    OwnedHandle::new_file(raw).ok_or_else(|| api_error("CreateFileW(NUL)"))
}

fn inheritable_security_attributes() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).expect("SECURITY_ATTRIBUTES size"),
        bInheritHandle: 1,
        lpSecurityDescriptor: std::ptr::null_mut(),
    }
}

struct AttributeList {
    storage: Vec<usize>,
    raw: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl AttributeList {
    fn for_handles(handles: &[HANDLE]) -> Result<Self, NativeError> {
        let mut bytes = 0;
        // SAFETY: the documented sizing call accepts a null list and writes the required size.
        let _ = unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &raw mut bytes)
        };
        if bytes == 0 {
            return Err(api_error("InitializeProcThreadAttributeList(size)"));
        }
        let words = bytes.div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; words];
        let raw = storage.as_mut_ptr().cast();
        // SAFETY: the aligned backing allocation has at least the requested byte count.
        if unsafe { InitializeProcThreadAttributeList(raw, 1, 0, &raw mut bytes) } == 0 {
            return Err(api_error("InitializeProcThreadAttributeList"));
        }
        // SAFETY: the initialized list and stable handle slice remain live through CreateProcessW.
        if unsafe {
            UpdateProcThreadAttribute(
                raw,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            let error = api_error("UpdateProcThreadAttribute(handle list)");
            // SAFETY: `raw` was initialized successfully above.
            unsafe { DeleteProcThreadAttributeList(raw) };
            return Err(error);
        }
        Ok(Self { storage, raw })
    }

    fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        debug_assert!(!self.storage.is_empty());
        self.raw
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: `raw` was initialized once and is deleted once before its storage is released.
        unsafe { DeleteProcThreadAttributeList(self.raw) };
    }
}
