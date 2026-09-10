//! Read only the URL-protocol key/marker metadata. Never read a command value,
//! launch a URL, create a registry key, or infer that an application opened.
use crate::NativeError;
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY_CLASSES_ROOT, KEY_QUERY_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolRegistration {
    KeyMissing,
    UrlMarkerMissing,
    UrlMarkerPresent,
    UnexpectedMarkerType(u32),
}

pub fn registration(scheme: &str) -> Result<ProtocolRegistration, NativeError> {
    let mut chars = scheme.chars();
    if scheme.len() > 64
        || !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return Err(NativeError::InvalidInput(
            "invalid URL scheme for registration lookup".into(),
        ));
    }
    let key_name: Vec<u16> = scheme.encode_utf16().chain([0]).collect();
    let mut key = std::ptr::null_mut();
    // SAFETY: A fixed predefined hive, terminated scheme, query-only rights and
    // valid writable output handle. This opens an existing key, never creates it.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            key_name.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &raw mut key,
        )
    };
    if matches!(opened, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        return Ok(ProtocolRegistration::KeyMissing);
    }
    if opened != ERROR_SUCCESS {
        return Err(api_status("RegOpenKeyExW(protocol)", opened));
    }
    let marker: Vec<u16> = "URL Protocol".encode_utf16().chain([0]).collect();
    let mut kind = 0;
    // SAFETY: key was successfully opened; marker is terminated; kind is writable.
    // Null data/size means only the type/existence is queried, with no value read.
    let queried = unsafe {
        RegQueryValueExW(
            key,
            marker.as_ptr(),
            std::ptr::null(),
            &raw mut kind,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    // SAFETY: This owns a newly opened non-predefined key and closes it once.
    let closed = unsafe { RegCloseKey(key) };
    let result = marker_result(queried, kind)?;
    if closed != ERROR_SUCCESS {
        return Err(api_status("RegCloseKey(protocol)", closed));
    }
    Ok(result)
}

fn marker_result(status: u32, kind: u32) -> Result<ProtocolRegistration, NativeError> {
    match status {
        ERROR_FILE_NOT_FOUND => Ok(ProtocolRegistration::UrlMarkerMissing),
        ERROR_SUCCESS if kind == REG_SZ => Ok(ProtocolRegistration::UrlMarkerPresent),
        ERROR_SUCCESS => Ok(ProtocolRegistration::UnexpectedMarkerType(kind)),
        other => Err(api_status("RegQueryValueExW(URL Protocol metadata)", other)),
    }
}

fn api_status(operation: &'static str, status: u32) -> NativeError {
    NativeError::Api {
        operation,
        source: std::io::Error::from_raw_os_error(i32::from_ne_bytes(status.to_ne_bytes())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_wrong_type_and_access_failure_are_not_registered_success() {
        assert_eq!(
            marker_result(ERROR_FILE_NOT_FOUND, 0).unwrap(),
            ProtocolRegistration::UrlMarkerMissing
        );
        assert_eq!(
            marker_result(ERROR_SUCCESS, REG_SZ).unwrap(),
            ProtocolRegistration::UrlMarkerPresent
        );
        assert_eq!(
            marker_result(ERROR_SUCCESS, 4).unwrap(),
            ProtocolRegistration::UnexpectedMarkerType(4)
        );
        assert!(
            marker_result(5, 0)
                .unwrap_err()
                .to_string()
                .contains("RegQueryValueExW")
        );
        for invalid in [
            "",
            "\\codex",
            "codex\\shell",
            "codex://",
            "codex\0",
            "1codex",
        ] {
            assert!(registration(invalid).is_err());
        }
    }
}
