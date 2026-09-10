pub const DISCORD_MAX_LEN: usize = 1_900;
const CHUNK_MARKER_BUDGET: usize = 32;
const TRUNCATION_SUFFIX: &str = "\n\n[truncated for Discord]";

#[must_use]
pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    assert!(limit > 0, "Discord message limit must be positive");
    let mut remaining = text.trim();
    if remaining.is_empty() {
        return vec!["(no output)".into()];
    }
    let mut chunks = Vec::new();
    while remaining.chars().count() > limit {
        let prefix_end = char_byte_index(remaining, limit);
        let prefix = &remaining[..prefix_end];
        let split_at = prefix
            .rfind('\n')
            .filter(|index| *index > 0)
            .unwrap_or(prefix_end);
        chunks.push(remaining[..split_at].trim().to_owned());
        remaining = remaining[split_at..].trim_start();
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_owned());
    }
    chunks
}

#[must_use]
pub fn split_delivery_chunks(text: &str, markers_enabled: bool) -> Vec<String> {
    if !markers_enabled {
        return split_message(text, DISCORD_MAX_LEN);
    }
    let chunks = split_message(text, DISCORD_MAX_LEN - CHUNK_MARKER_BUDGET);
    if chunks.len() <= 1 {
        return chunks;
    }
    let total = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| format!("[{}/{total}]\n{chunk}", index + 1))
        .collect()
}

/// Split a saved structured payload without trimming meaningful whitespace.
#[must_use]
pub fn split_exact_delivery_chunks(text: &str, markers_enabled: bool) -> Vec<String> {
    if text.is_empty() {
        return vec!["(no output)".into()];
    }
    let limit = DISCORD_MAX_LEN
        - if markers_enabled {
            CHUNK_MARKER_BUDGET
        } else {
            0
        };
    let mut remaining = text;
    let mut chunks = Vec::new();
    while !remaining.is_empty() {
        let end = char_byte_index(remaining, limit);
        chunks.push(remaining[..end].to_owned());
        remaining = &remaining[end..];
    }
    let total = chunks.len();
    if markers_enabled && total > 1 {
        chunks
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| format!("[{}/{total}]\n{chunk}", index + 1))
            .collect()
    } else {
        chunks
    }
}

#[must_use]
pub fn fit_single_message(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let available = limit.saturating_sub(TRUNCATION_SUFFIX.chars().count());
    let split_at = char_byte_index(text, available);
    format!("{}{TRUNCATION_SUFFIX}", text[..split_at].trim_end())
}

fn char_byte_index(text: &str, character_index: usize) -> usize {
    text.char_indices()
        .nth(character_index)
        .map_or(text.len(), |(index, _)| index)
}
