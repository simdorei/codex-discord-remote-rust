mod network;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cdr_remote_protocol::output::{CoreOutput, ImageEntry};
use thiserror::Error;
use tokio::sync::watch;

use crate::checkpoints::{CheckpointError, mutate};
use crate::files::{ProjectFileAccess, RemoteFileError, hex_digest};

pub const MAX_IMAGE_BYTES: usize = 5_242_880;
const MAX_IMAGE_RESULTS: usize = 200;

#[derive(Debug, Error)]
pub enum ImageError {
    #[error(transparent)]
    File(#[from] RemoteFileError),
    #[error(transparent)]
    Checkpoint(#[from] CheckpointError),
    #[error("{path}: {reason}")]
    Invalid { path: String, reason: String },
    #[error("<image-url>: {0}")]
    Network(String),
    #[error("image request was cancelled because the local bridge disconnected")]
    Cancelled,
    #[error("image request expired before it completed")]
    TimedOut,
}

pub fn save(
    access: &ProjectFileAccess,
    path: &str,
    data_base64: &str,
    overwrite: bool,
) -> Result<CoreOutput, ImageError> {
    let content = STANDARD
        .decode(data_base64)
        .map_err(|_| invalid(path, "image data is not valid base64"))?;
    save_bytes(access, path, &content, overwrite)
}

pub async fn save_from_url(
    access: &ProjectFileAccess,
    path: &str,
    url: &str,
    overwrite: bool,
    cancelled: watch::Receiver<bool>,
    budget: &cdr_core::deadline::RequestBudget,
) -> Result<CoreOutput, ImageError> {
    let content = network::download(url, cancelled, budget).await?;
    save_bytes(access, path, &content, overwrite)
}

pub fn list(
    access: &ProjectFileAccess,
    cancelled: &watch::Receiver<bool>,
) -> Result<CoreOutput, ImageError> {
    let mut images = Vec::new();
    for relative in access.scan_paths()? {
        ensure_active(cancelled)?;
        if images.len() >= MAX_IMAGE_RESULTS {
            break;
        }
        let path = relative.to_string_lossy().replace('\\', "/");
        if media_type(&path).is_none() {
            continue;
        }
        let Ok(content) = access.read_bytes(&path, MAX_IMAGE_BYTES) else {
            continue;
        };
        if let Ok(image) = image_entry(&path, &content) {
            images.push(image);
        }
    }
    Ok(CoreOutput::ImageList { images })
}

pub fn retrieve(
    access: &ProjectFileAccess,
    path: &str,
    cancelled: &watch::Receiver<bool>,
) -> Result<CoreOutput, ImageError> {
    ensure_active(cancelled)?;
    let content = access.read_bytes(path, MAX_IMAGE_BYTES)?;
    let image = image_entry(path, &content)?;
    Ok(CoreOutput::ImageRetrieve {
        image,
        data_base64: STANDARD.encode(content),
    })
}

fn save_bytes(
    access: &ProjectFileAccess,
    path: &str,
    content: &[u8],
    overwrite: bool,
) -> Result<CoreOutput, ImageError> {
    let image = image_entry(path, content)?;
    let exists = access.file_exists(path)?;
    if exists && !overwrite {
        return Err(invalid(path, "file already exists"));
    }
    let expected = if exists {
        Some(hex_digest(&access.read_bytes(path, MAX_IMAGE_BYTES)?))
    } else {
        None
    };
    let paths = [path.to_owned()];
    let ((), _) = mutate(access, "save image", &paths, |tracker| {
        access.write_bytes(path, content, MAX_IMAGE_BYTES, expected.as_deref())?;
        tracker.record_write(path, hex_digest(content));
        Ok(())
    })?;
    Ok(CoreOutput::ImageSave {
        image,
        sha256: hex_digest(content),
    })
}

fn image_entry(path: &str, content: &[u8]) -> Result<ImageEntry, ImageError> {
    if content.len() > MAX_IMAGE_BYTES {
        return Err(invalid(
            path,
            &format!("image exceeds {MAX_IMAGE_BYTES} bytes"),
        ));
    }
    let media_type = validate(path, content)?;
    Ok(ImageEntry {
        path: ProjectFileAccess::validate_path(path)?,
        media_type: media_type.into(),
        size_bytes: content.len() as u64,
    })
}

fn validate(path: &str, content: &[u8]) -> Result<&'static str, ImageError> {
    let Some((media_type, signatures)) = media_type(path) else {
        return Err(invalid(
            path,
            "supported types are PNG, JPEG, GIF, and WebP",
        ));
    };
    if !signatures
        .iter()
        .any(|signature| content.starts_with(signature))
    {
        return Err(invalid(path, "file content does not match its image type"));
    }
    if extension(path).is_some_and(|value| value.eq_ignore_ascii_case("webp"))
        && content.get(8..12) != Some(b"WEBP")
    {
        return Err(invalid(path, "file content does not match WebP"));
    }
    Ok(media_type)
}

type ImageType = (&'static str, &'static [&'static [u8]]);

fn media_type(path: &str) -> Option<ImageType> {
    match extension(path)?.to_ascii_lowercase().as_str() {
        "png" => Some(("image/png", &[b"\x89PNG\r\n\x1a\n"])),
        "jpg" | "jpeg" => Some(("image/jpeg", &[b"\xff\xd8\xff"])),
        "gif" => Some(("image/gif", &[b"GIF87a", b"GIF89a"])),
        "webp" => Some(("image/webp", &[b"RIFF"])),
        _ => None,
    }
}

fn extension(path: &str) -> Option<&str> {
    std::path::Path::new(path).extension()?.to_str()
}

fn ensure_active(cancelled: &watch::Receiver<bool>) -> Result<(), ImageError> {
    if *cancelled.borrow() {
        Err(ImageError::Cancelled)
    } else {
        Ok(())
    }
}

fn invalid(path: &str, reason: &str) -> ImageError {
    ImageError::Invalid {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}
