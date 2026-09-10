use cdr_codex_state::{ThreadInfo, normalize_workspace_path, strip_windows_extended_prefix};

pub(super) fn project(thread: &ThreadInfo) -> (String, String) {
    let cwd = strip_windows_extended_prefix(&thread.cwd);
    let parts = cwd
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let count = parts.len();
    if count >= 5
        && parts[count - 4].eq_ignore_ascii_case("documents")
        && parts[count - 3].eq_ignore_ascii_case("codex")
        && parts[count - 2].len() == 10
        && (parts[count - 1].starts_with("new-chat")
            || parts[count - 1].starts_with("chatgpt-conversation-"))
    {
        return ("codex:chats".into(), "채팅".into());
    }
    if cwd.is_empty() {
        return ("projectless:projectless".into(), "projectless".into());
    }
    (
        normalize_workspace_path(&cwd),
        parts.last().copied().unwrap_or("projectless").to_owned(),
    )
}

pub(super) fn channel_name(name: &str) -> String {
    let mut result = String::new();
    for ch in name.to_lowercase().chars() {
        let ch = if ch.is_ascii_alphanumeric() || ('가'..='힣').contains(&ch) || "._-".contains(ch)
        {
            ch
        } else {
            '-'
        };
        if ch != '-' || !result.ends_with('-') {
            result.push(ch);
        }
    }
    let result = result.trim_matches(['-', '.', '_']);
    let result = if result.starts_with("codex-") {
        result.to_owned()
    } else {
        format!("codex-{result}")
    };
    truncate(&result, 80)
        .trim_matches(['-', '.', '_'])
        .to_owned()
}

pub(super) fn thread_name(name: &str, id: &str) -> String {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return format!("codex-{}", id.chars().take(8).collect::<String>());
    }
    truncate(&name, 90)
}

fn truncate(value: &str, limit: usize) -> String {
    let mut remaining = limit;
    value
        .chars()
        .take_while(|ch| {
            if ch.len_utf16() > remaining {
                return false;
            }
            remaining -= ch.len_utf16();
            true
        })
        .collect()
}
