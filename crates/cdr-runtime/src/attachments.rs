use std::path::{Path, PathBuf};

use thiserror::Error;
use twilight_model::channel::Message;

mod download;
use download::download_attachment;

use crate::config::RuntimeConfig;

const PREVIEW_CHARS: usize = 12_000;
const TEXT_EXTENSIONS: &[&str] = &[
    "bat", "cmd", "css", "csv", "html", "ini", "js", "json", "log", "md", "ps1", "py", "rs", "sh",
    "toml", "ts", "tsx", "txt", "xml", "yaml", "yml",
];

#[derive(Debug, Error)]
pub enum AttachmentError {
    #[error("attachment filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("required new-thread attachment preparation failed: {0}")]
    Required(String),
}

pub async fn enrich_message_attachments(
    message: &Message,
    base_prompt: &str,
    config: &RuntimeConfig,
    attachment_root: &Path,
    client: &reqwest::Client,
) -> Result<String, AttachmentError> {
    enrich_attachments(message, base_prompt, config, attachment_root, client, false).await
}

pub async fn enrich_new_message_attachments(
    message: &Message,
    base_prompt: &str,
    config: &RuntimeConfig,
    attachment_root: &Path,
    client: &reqwest::Client,
) -> Result<String, AttachmentError> {
    enrich_attachments(message, base_prompt, config, attachment_root, client, true).await
}

async fn enrich_attachments(
    message: &Message,
    base_prompt: &str,
    config: &RuntimeConfig,
    attachment_root: &Path,
    client: &reqwest::Client,
    require_all: bool,
) -> Result<String, AttachmentError> {
    if require_all && !message.attachments.is_empty() && !config.attachments_enabled {
        return Err(AttachmentError::Required(
            "attachments are disabled; no new thread started".into(),
        ));
    }
    if message.attachments.is_empty() || !config.attachments_enabled {
        return Ok(base_prompt.into());
    }
    let directory = attachment_root
        .join(message.channel_id.to_string())
        .join(message.id.to_string());
    tokio::fs::create_dir_all(&directory).await?;
    let mut details = Vec::new();
    let mut previews = Vec::new();
    for (offset, attachment) in message.attachments.iter().enumerate() {
        let index = offset + 1;
        match download_attachment(
            index,
            attachment,
            &directory,
            config.attachment_max_bytes,
            config.attachment_text_inline_max_bytes,
            client,
            require_all,
        )
        .await
        {
            Ok((detail, preview)) => {
                details.push(detail);
                if let Some(preview) = preview {
                    previews.push(preview);
                }
            }
            Err(error) => {
                if require_all {
                    return Err(AttachmentError::Required(format!(
                        "{}: {error}",
                        sanitize_filename(&attachment.filename, index)
                    )));
                }
                eprintln!(
                    "attachment_download_failed message={} filename={} error={error}",
                    message.id,
                    sanitize_filename(&attachment.filename, index)
                );
                details.push(format!(
                    "{index}. {} failed to save: {error}",
                    sanitize_filename(&attachment.filename, index)
                ));
            }
        }
    }
    Ok(render_attachment_prompt(base_prompt, &details, &previews))
}

#[must_use]
pub fn sanitize_filename(filename: &str, index: usize) -> String {
    let path = PathBuf::from(filename);
    let leaf = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let safe = leaf
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ' ' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let safe = safe
        .trim_matches([' ', '.'])
        .chars()
        .take(120)
        .collect::<String>();
    if safe.is_empty() {
        format!("attachment-{index}")
    } else {
        safe
    }
}

#[must_use]
pub fn is_text_attachment(filename: &str, content_type: Option<&str>) -> bool {
    content_type.is_some_and(|value| value.to_ascii_lowercase().starts_with("text/"))
        || Path::new(filename)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| TEXT_EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
}

#[must_use]
pub fn render_attachment_prompt(
    base_prompt: &str,
    details: &[String],
    previews: &[(String, String)],
) -> String {
    if details.is_empty() {
        return base_prompt.into();
    }
    let mut lines = vec![
        base_prompt.trim().to_owned(),
        String::new(),
        "Discord attachments saved locally:".into(),
        details.join("\n"),
    ];
    if !previews.is_empty() {
        lines.extend([String::new(), "Attachment text previews:".into()]);
        for (filename, preview) in previews {
            lines.push(format!("--- {filename} ---\n```text\n{preview}\n```"));
        }
    }
    lines.join("\n").trim().to_owned()
}
