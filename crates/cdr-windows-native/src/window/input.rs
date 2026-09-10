use std::mem::size_of;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
};

use super::identity::{activate_window, require_active};
use crate::error::{NativeError, api_error};

const MODIFIERS: &[(&str, u16)] = &[("CTRL", 0x11), ("SHIFT", 0x10), ("ALT", 0x12)];

pub fn type_text(window_id: u64, process_id: u32, text: &str) -> Result<bool, NativeError> {
    let activated = activate_window(window_id, process_id)?;
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
    require_active(window_id, process_id)?;
    Ok(activated)
}

pub fn press_keys(
    window_id: u64,
    process_id: u32,
    keys: &[String],
) -> Result<(bool, Vec<String>), NativeError> {
    let normalized = normalize_keys(keys)?;
    let activated = activate_window(window_id, process_id)?;
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
    if let Err(error) = send(&inputs) {
        best_effort_release(&codes);
        return Err(error);
    }
    require_active(window_id, process_id)?;
    Ok((activated, normalized))
}

pub fn normalize_keys(keys: &[String]) -> Result<Vec<String>, NativeError> {
    let normalized = keys
        .iter()
        .map(|key| key.trim().to_ascii_uppercase().replace(' ', ""))
        .collect::<Vec<_>>();
    if normalized.iter().any(String::is_empty)
        || normalized
            .iter()
            .enumerate()
            .any(|(index, key)| normalized[..index].contains(key))
    {
        return Err(invalid("terminal key names must be unique and non-empty"));
    }
    if normalized
        .iter()
        .any(|key| ["WIN", "WINDOWS", "META", "COMMAND"].contains(&key.as_str()))
    {
        return Err(invalid("system-wide keys are not terminal-bound"));
    }
    let non_modifiers = normalized
        .iter()
        .filter(|key| !MODIFIERS.iter().any(|(name, _)| name == &key.as_str()))
        .collect::<Vec<_>>();
    if non_modifiers.len() != 1 {
        return Err(invalid("one non-modifier terminal key is required"));
    }
    for key in &normalized {
        let _ = key_code(key)?;
    }
    Ok(MODIFIERS
        .iter()
        .filter(|(name, _)| normalized.iter().any(|key| key == name))
        .map(|(name, _)| (*name).to_owned())
        .chain(non_modifiers.into_iter().cloned())
        .collect())
}

fn key_code(key: &str) -> Result<u16, NativeError> {
    if let Some((_, code)) = MODIFIERS.iter().find(|(name, _)| *name == key) {
        return Ok(*code);
    }
    let named = match key {
        "BACKSPACE" => Some(0x08),
        "TAB" => Some(0x09),
        "ENTER" => Some(0x0D),
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
        "INSERT" => Some(0x2D),
        "DELETE" => Some(0x2E),
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
        && (1..=12).contains(&number)
    {
        return Ok(0x6F + number);
    }
    Err(invalid(&format!("unsupported terminal key: {key}")))
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

fn best_effort_release(codes: &[u16]) {
    let releases = codes
        .iter()
        .rev()
        .map(|code| keyboard_input(*code, 0, KEYEVENTF_KEYUP))
        .collect::<Vec<_>>();
    let _ = send(&releases);
}

fn invalid(message: &str) -> NativeError {
    NativeError::InvalidInput(message.into())
}

#[cfg(test)]
mod tests {
    use super::normalize_keys;

    #[test]
    fn normalizes_terminal_bound_keys_and_rejects_system_keys() {
        assert_eq!(
            normalize_keys(&["l".into(), "ctrl".into()]).expect("keys"),
            ["CTRL", "L"]
        );
        assert!(normalize_keys(&["WIN".into(), "R".into()]).is_err());
        assert!(normalize_keys(&["CTRL".into(), "L".into(), "K".into()]).is_err());
    }
}
