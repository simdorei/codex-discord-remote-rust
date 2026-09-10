use std::ptr::{null, null_mut};
use std::slice;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use crate::error::{NativeError, api_error};

pub fn protect_current_user(plaintext: &[u8]) -> Result<Vec<u8>, NativeError> {
    transform(plaintext, true)
}

pub fn unprotect_current_user(ciphertext: &[u8]) -> Result<Vec<u8>, NativeError> {
    transform(ciphertext, false)
}

fn transform(input: &[u8], protect: bool) -> Result<Vec<u8>, NativeError> {
    let input_len = u32::try_from(input.len())
        .map_err(|_| NativeError::InvalidInput("DPAPI input is too large".into()))?;
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input_len,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output_blob = CRYPT_INTEGER_BLOB::default();
    // SAFETY: The input blob points to `input` for the duration of the call and
    // the output blob is initialized by DPAPI. All optional pointers are null.
    let succeeded = unsafe {
        if protect {
            CryptProtectData(
                &raw const input_blob,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output_blob,
            )
        } else {
            CryptUnprotectData(
                &raw const input_blob,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output_blob,
            )
        }
    };
    if succeeded == 0 {
        return Err(api_error(if protect {
            "CryptProtectData"
        } else {
            "CryptUnprotectData"
        }));
    }
    copy_and_free(output_blob)
}

fn copy_and_free(blob: CRYPT_INTEGER_BLOB) -> Result<Vec<u8>, NativeError> {
    if blob.pbData.is_null() {
        return if blob.cbData == 0 {
            Ok(Vec::new())
        } else {
            Err(NativeError::InvalidInput(
                "DPAPI returned an invalid output buffer".into(),
            ))
        };
    }
    // SAFETY: A successful DPAPI call returns `cbData` initialized bytes at
    // `pbData`, allocated with LocalAlloc. We copy before releasing the buffer.
    let value = unsafe { slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec() };
    // SAFETY: `pbData` is the exact allocation returned by DPAPI and is freed once.
    if !unsafe { LocalFree(blob.pbData.cast()) }.is_null() {
        return Err(api_error("LocalFree"));
    }
    Ok(value)
}
