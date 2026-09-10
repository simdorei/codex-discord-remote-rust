use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt as _;

use crate::NativeError;

pub fn wide(value: &OsStr, field: &'static str) -> Result<Vec<u16>, NativeError> {
    let mut encoded = value.encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(NativeError::InteriorNul(field));
    }
    encoded.push(0);
    Ok(encoded)
}

pub fn command_line(executable: &OsStr, arguments: &[String]) -> Result<Vec<u16>, NativeError> {
    let executable = executable.to_string_lossy();
    if executable.contains('\0') || arguments.iter().any(|value| value.contains('\0')) {
        return Err(NativeError::InteriorNul("command line"));
    }
    let line = std::iter::once(executable.as_ref())
        .chain(arguments.iter().map(String::as_str))
        .map(quote_argument)
        .collect::<Vec<_>>()
        .join(" ");
    wide(OsStr::new(&line), "command line")
}

pub fn environment_block(environment: &HashMap<String, String>) -> Result<Vec<u16>, NativeError> {
    let mut block = Vec::new();
    for (name, value) in normalized_entries(environment) {
        if name.contains('=') {
            return Err(NativeError::InvalidInput(
                "environment variable name contains '='".into(),
            ));
        }
        if name.contains('\0') || value.contains('\0') {
            return Err(NativeError::InteriorNul("environment"));
        }
        block.extend(OsStr::new(&format!("{name}={value}")).encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

pub(super) fn normalized_entries(environment: &HashMap<String, String>) -> Vec<(&String, &String)> {
    let mut entries = environment.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| {
        left.to_ascii_uppercase()
            .cmp(&right.to_ascii_uppercase())
            .then_with(|| left.cmp(right))
    });
    let mut normalized: Vec<(&String, &String)> = Vec::with_capacity(entries.len());
    for entry in entries {
        if normalized
            .last()
            .is_some_and(|(name, _)| name.eq_ignore_ascii_case(entry.0))
        {
            continue;
        }
        normalized.push(entry);
    }
    normalized
}

fn quote_argument(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.extend(std::iter::repeat_n('\\', backslashes));
            quoted.push(character);
            backslashes = 0;
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{environment_block, normalized_entries, quote_argument};

    #[test]
    fn quotes_windows_arguments_without_changing_backslashes() {
        assert_eq!(quote_argument("plain"), "plain");
        assert_eq!(quote_argument("two words"), "\"two words\"");
        assert_eq!(quote_argument(r#"a"b"#), "\"a\\\"b\"");
        assert_eq!(quote_argument(r"ends \"), "\"ends \\\\\"");
    }

    #[test]
    fn empty_environment_is_double_nul_terminated() {
        assert_eq!(environment_block(&HashMap::new()).unwrap(), [0, 0]);
    }

    #[test]
    fn environment_names_are_normalized_case_insensitively() {
        let environment = HashMap::from([
            ("path".into(), "lower-path".into()),
            ("PATH".into(), "canonical-path".into()),
            ("ComSpec".into(), "mixed-comspec".into()),
            ("COMSPEC".into(), "canonical-comspec".into()),
        ]);
        let entries = normalized_entries(&environment);
        assert_eq!(
            entries
                .into_iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect::<Vec<_>>(),
            [("COMSPEC", "canonical-comspec"), ("PATH", "canonical-path")]
        );
    }
}
