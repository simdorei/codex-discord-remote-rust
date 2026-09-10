use std::mem::size_of;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
};

use super::identity::require_active;
use crate::error::{NativeError, api_error};

pub fn type_device_text(window_id: u64, process_id: u32, text: &str) -> Result<(), NativeError> {
    require_active(window_id, process_id)?;
    let inputs = text
        .encode_utf16()
        .flat_map(|unit| {
            [
                keyboard_input(0, unit, KEYEVENTF_UNICODE),
                keyboard_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
            ]
        })
        .collect::<Vec<_>>();
    send(&inputs)?;
    require_active(window_id, process_id)
}

pub fn press_device_keys(
    window_id: u64,
    process_id: u32,
    keys: &[String],
) -> Result<Vec<String>, NativeError> {
    require_active(window_id, process_id)?;
    let normalized = normalize_device_keys(keys)?;
    let codes = normalized
        .iter()
        .map(|key| key_code(key))
        .collect::<Result<Vec<_>, _>>()?;
    let mut inputs = codes
        .iter()
        .map(|code| keyboard_input(*code, 0, 0))
        .collect::<Vec<_>>();
    inputs.extend(
        codes
            .iter()
            .rev()
            .map(|code| keyboard_input(*code, 0, KEYEVENTF_KEYUP)),
    );
    send(&inputs)?;
    require_active(window_id, process_id)?;
    Ok(normalized)
}

fn normalize_device_keys(keys: &[String]) -> Result<Vec<String>, NativeError> {
    let normalized = keys
        .iter()
        .map(|key| {
            let value = key.trim().to_ascii_uppercase().replace(' ', "");
            match value.as_str() {
                "CONTROL" => "CTRL".into(),
                "ESCAPE" => "ESC".into(),
                "WINDOWS" | "META" => "WIN".into(),
                _ => value,
            }
        })
        .collect::<Vec<String>>();
    if normalized.iter().any(String::is_empty)
        || normalized
            .iter()
            .enumerate()
            .any(|(index, key)| normalized[..index].contains(key))
    {
        return Err(invalid("computer key names must be unique and non-empty"));
    }
    if normalized.len() == 3
        && ["CTRL", "ALT", "DELETE"]
            .iter()
            .all(|key| normalized.iter().any(|value| value == key))
    {
        return Err(invalid("Ctrl+Alt+Delete requires direct user control"));
    }
    for key in &normalized {
        let _ = key_code(key)?;
    }
    Ok(normalized)
}

fn key_code(key: &str) -> Result<u16, NativeError> {
    let named = match key {
        "BACKSPACE" => Some(0x08),
        "TAB" => Some(0x09),
        "ENTER" => Some(0x0D),
        "SHIFT" => Some(0x10),
        "CTRL" => Some(0x11),
        "ALT" => Some(0x12),
        "PAUSE" => Some(0x13),
        "CAPSLOCK" => Some(0x14),
        "ESC" => Some(0x1B),
        "SPACE" => Some(0x20),
        "PAGEUP" => Some(0x21),
        "PAGEDOWN" => Some(0x22),
        "END" => Some(0x23),
        "HOME" => Some(0x24),
        "LEFT" => Some(0x25),
        "UP" => Some(0x26),
        "RIGHT" => Some(0x27),
        "DOWN" => Some(0x28),
        "PRINTSCREEN" => Some(0x2C),
        "INSERT" => Some(0x2D),
        "DELETE" => Some(0x2E),
        "WIN" => Some(0x5B),
        _ => None,
    };
    if let Some(code) = named {
        return Ok(code);
    }
    let bytes = key.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_uppercase() || bytes[0].is_ascii_digit()) {
        return Ok(u16::from(bytes[0]));
    }
    if let Some(number) = key
        .strip_prefix('F')
        .and_then(|value| value.parse::<u16>().ok())
        && (1..=24).contains(&number)
    {
        return Ok(0x6F + number);
    }
    Err(invalid(&format!("unsupported computer key: {key}")))
}

fn keyboard_input(virtual_key: u16, scan_code: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), NativeError> {
    // SAFETY: the contiguous INPUT slice remains live and its element size is exact.
    let sent = unsafe {
        SendInput(
            u32::try_from(inputs.len()).unwrap_or(u32::MAX),
            inputs.as_ptr(),
            i32::try_from(size_of::<INPUT>()).expect("INPUT size"),
        )
    };
    if usize::try_from(sent).unwrap_or_default() == inputs.len() {
        Ok(())
    } else {
        Err(api_error("SendInput"))
    }
}

fn invalid(message: &str) -> NativeError {
    NativeError::InvalidInput(message.into())
}

#[cfg(test)]
mod tests {
    use super::normalize_device_keys;

    #[test]
    fn device_keys_support_aliases_but_reject_secure_attention() {
        assert_eq!(
            normalize_device_keys(&["control".into(), "a".into()]).unwrap(),
            ["CTRL", "A"]
        );
        assert!(normalize_device_keys(&["CTRL".into(), "ALT".into(), "DELETE".into()]).is_err());
    }
}
