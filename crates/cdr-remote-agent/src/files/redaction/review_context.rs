//! Narrow, occurrence-based exceptions to the opaque-token heuristic.
//! Explicit credential masking runs first; unknown contexts remain masked.
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;

use serde_json::Value;

pub(super) fn allowed_ranges(text: &str, path: &Path) -> Vec<Range<usize>> {
    let mut ranges = match path.extension().and_then(|value| value.to_str()) {
        Some("py") => python_ranges(text),
        Some("json") => json_ranges(text).unwrap_or_default(),
        _ => Vec::new(),
    };
    ranges.sort_by_key(|range| range.start);
    ranges
}

fn sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "secret",
        "password",
        "passwd",
        "credential",
        "token",
        "api_key",
        "apikey",
        "api-key",
        "authorization",
        "private_key",
        "private-key",
        "access_key",
    ]
    .iter()
    .any(|word| key.contains(word))
        || key == "pwd"
}

fn local_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("://")
        && !value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '?' | '#'))
        && (value.contains(['/', '\\'])
            || value.rsplit_once('.').is_some_and(|(_, ext)| {
                ["py", "rs", "json", "md", "exe", "ps1", "txt", "log"].contains(&ext)
            }))
}

fn digest(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn public_ffi_signature(value: &str, suffix: &str) -> bool {
    // Public Win32 names in ctypes signature tables are code, not credentials.
    // Do not infer that arbitrary dictionary keys or string literals are safe.
    value == "InitializeProcThreadAttributeList"
        && suffix
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
            .is_some_and(|tail| tail.starts_with("(w.") || tail.starts_with("(ctypes."))
}

fn python_ranges(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'#' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if matches!(bytes[index], b'\'' | b'"') {
            let quote = bytes[index];
            let begin = index;
            let triple = bytes.get(index..index + 3) == Some(&[quote; 3]);
            let width = if triple { 3 } else { 1 };
            index += width;
            let body = index;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes.get(index..index + width) == Some(&[quote; 3][..width]) {
                    if !triple
                        && ((text[..begin].trim_end().ends_with('/')
                            && local_path(&text[body..index]))
                            || public_ffi_signature(&text[body..index], &text[index + width..]))
                    {
                        ranges.push(body..index);
                    }
                    index += width;
                    break;
                } else {
                    index += 1;
                }
            }
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let begin = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let prefix = text[..begin].trim_end();
            let suffix = text[index..].trim_start();
            let declaration = prefix.ends_with("def") || prefix.ends_with("class");
            if prefix.ends_with('.') || (declaration && suffix.starts_with(['(', ':'])) {
                ranges.push(begin..index);
            }
        } else {
            index += 1;
        }
    }
    ranges
}

#[derive(Clone, Copy, Default)]
struct Context {
    sensitive: bool,
    digest_length: Option<usize>,
    source_hashes: bool,
    path: bool,
}

struct JsonWalk<'a> {
    text: &'a str,
    cursor: usize,
    ranges: Vec<Range<usize>>,
}

fn json_ranges(text: &str) -> Option<Vec<Range<usize>>> {
    // Windows-produced UTF-8 JSON can carry a BOM. Keep its bytes in the
    // returned text and original SHA while parsing after it.
    let start = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let value: Value = serde_json::from_str(&text[start..]).ok()?;
    let mut walk = JsonWalk {
        text,
        cursor: start,
        ranges: Vec::new(),
    };
    walk.value(&value, Context::default())?;
    walk.space();
    (walk.cursor == text.len()).then_some(walk.ranges)
}

impl JsonWalk<'_> {
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
    }

    fn take(&mut self, byte: u8) -> Option<()> {
        self.space();
        if self.text.as_bytes().get(self.cursor) != Some(&byte) {
            return None;
        }
        self.cursor += 1;
        Some(())
    }

    fn string(&mut self) -> Option<(String, Range<usize>)> {
        self.space();
        let start = self.cursor;
        let mut stream =
            serde_json::Deserializer::from_str(&self.text[start..]).into_iter::<String>();
        let value = stream.next()?.ok()?;
        self.cursor += stream.byte_offset();
        Some((value, start + 1..self.cursor - 1))
    }

    fn value(&mut self, value: &Value, context: Context) -> Option<()> {
        self.space();
        match value {
            Value::Object(object) => {
                self.take(b'{')?;
                let mut seen = BTreeSet::new();
                let file_record = ["path", "source"].iter().any(|key| {
                    object
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(local_path)
                });
                for item in 0..object.len() {
                    if item > 0 {
                        self.take(b',')?;
                    }
                    let (key, key_range) = self.string()?;
                    if !seen.insert(key.clone()) {
                        return None;
                    }
                    self.take(b':')?;
                    let child = object.get(&key)?;
                    let hidden = context.sensitive || sensitive(&key);
                    let source_entry = context.source_hashes && local_path(&key);
                    if source_entry && !hidden {
                        self.ranges.push(key_range);
                    }
                    let hash = if (key == "sha256" && file_record) || source_entry {
                        Some(64)
                    } else if key == "head"
                        && object.contains_key("repository")
                        && object.get("files").is_some_and(Value::is_array)
                    {
                        Some(40)
                    } else {
                        None
                    };
                    self.value(
                        child,
                        Context {
                            sensitive: hidden,
                            digest_length: hash,
                            source_hashes: key == "source_hashes",
                            path: ["path", "source", "snapshot", "repository", "cwd"]
                                .contains(&key.as_str()),
                        },
                    )?;
                }
                self.take(b'}')?;
            }
            Value::Array(values) => {
                self.take(b'[')?;
                for (index, item) in values.iter().enumerate() {
                    if index > 0 {
                        self.take(b',')?;
                    }
                    self.value(
                        item,
                        Context {
                            sensitive: context.sensitive,
                            ..Context::default()
                        },
                    )?;
                }
                self.take(b']')?;
            }
            Value::String(expected) => {
                let (value, range) = self.string()?;
                if &value != expected {
                    return None;
                }
                if !context.sensitive
                    && (context
                        .digest_length
                        .is_some_and(|length| digest(&value, length))
                        || (context.path && local_path(&value)))
                {
                    self.ranges.push(range);
                }
            }
            _ => {
                let mut stream = serde_json::Deserializer::from_str(&self.text[self.cursor..])
                    .into_iter::<Value>();
                if &stream.next()?.ok()? != value {
                    return None;
                }
                self.cursor += stream.byte_offset();
            }
        }
        Some(())
    }
}
