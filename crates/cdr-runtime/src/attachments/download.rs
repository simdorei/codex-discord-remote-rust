use super::{PREVIEW_CHARS, is_text_attachment, sanitize_filename};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, path::Path};
use tokio::io::AsyncWriteExt;
use twilight_model::channel::Attachment;

pub(super) async fn download_attachment(
    index: usize,
    attachment: &Attachment,
    directory: &Path,
    max_bytes: u64,
    inline_max_bytes: u64,
    client: &reqwest::Client,
    require_all: bool,
) -> Result<(String, Option<(String, String)>), String> {
    let filename = sanitize_filename(&attachment.filename, index);
    if attachment.size > max_bytes {
        if require_all {
            return Err(format!(
                "file is {} bytes; limit is {max_bytes} bytes",
                attachment.size
            ));
        }
        return Ok((
            format!(
                "{index}. {filename} skipped: file is {} bytes; limit is {max_bytes} bytes.",
                attachment.size
            ),
            None,
        ));
    }
    let destination = directory.join(format!("{index:02}-{filename}"));
    let response = client
        .get(&attachment.url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.without_url().to_string())?;
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes)
    {
        if require_all {
            return Err(format!("response exceeds {max_bytes} bytes"));
        }
        return Ok((
            format!("{index}. {filename} skipped: response exceeds {max_bytes} bytes."),
            None,
        ));
    }
    let mut file = tokio::fs::File::create(&destination)
        .await
        .map_err(|error| error.to_string())?;
    let mut stream = response.bytes_stream();
    let mut written = 0_u64;
    let mut digest = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.without_url().to_string())?;
        written = written.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if written > max_bytes {
            drop(file);
            if let Err(error) = tokio::fs::remove_file(&destination).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(format!("oversized partial-file cleanup failed: {error}"));
            }
            if require_all {
                return Err(format!("download exceeded {max_bytes} bytes"));
            }
            return Ok((
                format!("{index}. {filename} skipped: download exceeded {max_bytes} bytes."),
                None,
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| error.to_string())?;
        digest.update(&chunk);
    }
    file.flush().await.map_err(|error| error.to_string())?;
    if require_all {
        file.sync_all().await.map_err(|error| error.to_string())?;
    }
    let content_type = attachment.content_type.as_deref().unwrap_or("-");
    let mut detail = format!(
        "{index}. {filename}\n   path: {}\n   content_type: {content_type}\n   size_bytes: {written}",
        destination.display()
    );
    if require_all {
        write!(detail, "\n   sha256: {}", hex::encode(digest.finalize()))
            .map_err(|error| error.to_string())?;
    }
    let preview = if written <= inline_max_bytes
        && is_text_attachment(&filename, attachment.content_type.as_deref())
    {
        text_preview(&destination, &filename, require_all).await?
    } else {
        None
    };
    Ok((detail, preview))
}

async fn text_preview(
    destination: &Path,
    filename: &str,
    required: bool,
) -> Result<Option<(String, String)>, String> {
    let bytes = match tokio::fs::read(destination).await {
        Ok(bytes) => bytes,
        Err(error) if required => {
            return Err(format!("text attachment preview read failed: {error}"));
        }
        Err(_) => return Ok(None),
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut preview = text.chars().take(PREVIEW_CHARS).collect::<String>();
    if text.chars().count() > PREVIEW_CHARS {
        preview.push_str("\n\n[truncated]");
    }
    Ok(Some((filename.into(), preview)))
}
