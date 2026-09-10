//! Delete the exact file that a caller has opened and independently validated.
use std::{fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
};

pub fn delete_open_file(file: File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    let size =
        u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO>()).map_err(io::Error::other)?;
    // SAFETY: File owns a live handle throughout this call. info has the exact
    // documented structure and size for FileDispositionInfo; the API copies it.
    let success = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&raw const info).cast(),
            size,
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    drop(file);
    Ok(())
}
