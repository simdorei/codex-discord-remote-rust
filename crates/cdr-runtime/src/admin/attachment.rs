//! One-shot attachment upload. No gateway, store migrations or automatic retry.
use super::{args::Args, project_environment};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
pub mod target;
pub mod transport;

pub struct UploadFile {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}
pub struct Upload {
    pub content: String,
    pub files: Vec<UploadFile>,
}
const MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(format!(
            "attachment input is not a regular file: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "attachment input exceeds local safety limit: {}",
            path.display()
        ));
    }
    Ok(bytes)
}

pub fn prepare(
    content: &str,
    content_file: Option<&Path>,
    files: &[PathBuf],
) -> Result<Upload, String> {
    if files.is_empty() {
        return Err("at least one attachment is required".into());
    }
    if files.len() > 10 {
        return Err("at most 10 attachments can be sent in one request".into());
    }
    let content = match content_file {
        Some(path) => String::from_utf8(bounded_read(path, 1024 * 1024)?)
            .map_err(|_| "message content file must be UTF-8")?,
        None => content.to_owned(),
    };
    let content = content.trim().to_owned();
    if content.encode_utf16().count() > 2000 {
        return Err("Discord message content exceeds 2000 UTF-16 units".into());
    }
    let mut remaining = MAX_TOTAL_BYTES;
    let files = files
        .iter()
        .map(|path| {
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
                .ok_or("attachment filename must be nonempty UTF-8 without control characters")?
                .to_owned();
            let bytes = bounded_read(path, remaining)?;
            remaining -= bytes.len() as u64;
            Ok(UploadFile {
                filename,
                content_type: mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string(),
                bytes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Upload { content, files })
}

pub(super) async fn run(args: &Args, root: &Path) -> Result<String, String> {
    let targets = ["--channel-id", "--thread-ref", "--work-thread"]
        .into_iter()
        .filter_map(|key| args.value(key).map(|value| (key, value)))
        .collect::<Vec<_>>();
    if targets.len() != 1 || targets[0].1.trim().is_empty() {
        return Err(
            "specify exactly one target: --channel-id, --thread-ref, or --work-thread".into(),
        );
    }
    if args.files.is_empty() {
        return Err("at least one attachment is required".into());
    }
    let env = project_environment(root)?;
    let token = env
        .get("DISCORD_BOT_TOKEN")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or("DISCORD_BOT_TOKEN is missing from environment or .env")?;
    transport::validate_token(token)?;
    let files = args.files.iter().map(PathBuf::from).collect::<Vec<_>>();
    let upload = prepare(
        args.value("--content").unwrap_or(""),
        args.value("--content-file").map(Path::new),
        &files,
    )?;
    let target = if targets[0].0 == "--channel-id" {
        target::Target::channel(targets[0].1)?
    } else {
        target::resolve(&env, root, targets[0].1)?
    };
    let receipt = transport::send("https://discord.com/api/v10", token, &target, upload).await?;
    Ok(format!(
        "DISCORD_ATTACHMENT_SENT\nmessage_id={}\nchannel_id={}\nattachments={}",
        receipt.message_id,
        receipt.channel_id,
        receipt.filenames.join(",")
    ))
}
