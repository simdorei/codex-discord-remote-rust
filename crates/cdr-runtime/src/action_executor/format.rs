use cdr_codex_state::{ThreadInfo, workspace_ref_map};

use super::ActionResult;

pub(super) const HELP: &str = include_str!("help.txt");
pub(super) const INTERVIEW_HEADER: &str = include_str!("interview_prompt.txt");

#[cfg(test)]
#[path = "help_connected_tests.rs"]
mod help_connected_tests;

pub(super) fn immediate(text: impl Into<String>) -> ActionResult {
    ActionResult {
        text: text.into(),
        waits_for_final: false,
        ui: None,
    }
}

pub(super) fn format_threads(threads: &[ThreadInfo], selected: Option<&str>, limit: u32) -> String {
    let refs = workspace_ref_map(threads);
    threads
        .iter()
        .enumerate()
        .take(if limit == 0 {
            usize::MAX
        } else {
            limit as usize
        })
        .map(|(index, thread)| {
            let marker = if Some(thread.id.as_str()) == selected {
                '*'
            } else {
                ' '
            };
            let reference = refs
                .get(&thread.id)
                .cloned()
                .unwrap_or_else(|| workspace_name(thread));
            format!(
                "{marker}{} | {reference} | {} | {}",
                index + 1,
                thread.id,
                thread.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn workspace_name(thread: &ThreadInfo) -> String {
    thread
        .cwd
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("-")
        .into()
}
