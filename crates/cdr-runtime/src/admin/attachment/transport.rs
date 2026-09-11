use super::{Upload, target::Target};
use reqwest::{
    Client, Response, header,
    multipart::{Form, Part},
};
use serde_json::{Value, json};

#[derive(Debug, PartialEq, Eq)]
pub struct Receipt {
    pub message_id: String,
    pub channel_id: String,
    pub filenames: Vec<String>,
}

pub fn validate_token(token: &str) -> Result<(), String> {
    if token.is_empty() || token.chars().any(char::is_control) {
        return Err("Discord bot token is empty or contains invalid control characters".into());
    }
    Ok(())
}
pub async fn send(
    base_url: &str,
    token: &str,
    target: &Target,
    upload: Upload,
) -> Result<Receipt, String> {
    validate_token(token)?;
    super::super::setup::validate_id(&target.channel_id)?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!(
        "{}/channels/{}",
        base_url.trim_end_matches('/'),
        target.channel_id
    );
    let authorization = format!("Bot {token}");
    let request = |url: String| {
        client
            .get(url)
            .header(header::AUTHORIZATION, &authorization)
            .header(
                header::USER_AGENT,
                "DiscordBot (https://github.com/simdorei/codex-discord-remote-rust, 1.0)",
            )
    };
    let response = request(url.clone())
        .send()
        .await
        .map_err(|e| format!("Discord channel access check failed: {}", e.without_url()))?;
    let status = response.status();
    let bytes = read_response(response).await?;
    if !status.is_success() {
        let hint = if target.mirrored {
            "; mirror mapping stale: run !mirror check, then !mirror sync"
        } else {
            ""
        };
        return Err(format!(
            "Discord channel {} is not accessible: HTTP {status}: {}{hint}",
            target.channel_id,
            diagnostic(&bytes, token)
        ));
    }
    let filenames = upload
        .files
        .iter()
        .map(|file| file.filename.clone())
        .collect::<Vec<_>>();
    let payload = json!({"content":upload.content,"allowed_mentions":{"parse":[]},"attachments":filenames.iter().enumerate().map(|(id,filename)|json!({"id":id,"filename":filename})).collect::<Vec<_>>()});
    let mut form = Form::new().part(
        "payload_json",
        Part::text(payload.to_string())
            .mime_str("application/json; charset=utf-8")
            .map_err(|e| e.to_string())?,
    );
    for (index, file) in upload.files.into_iter().enumerate() {
        let part = Part::bytes(file.bytes)
            .file_name(file.filename)
            .mime_str(&file.content_type)
            .map_err(|e| e.to_string())?;
        form = form.part(format!("files[{index}]"), part);
    }
    let response = client
        .post(format!("{url}/messages"))
        .header(header::AUTHORIZATION, &authorization)
        .header(
            header::USER_AGENT,
            "DiscordBot (https://github.com/simdorei/codex-discord-remote-rust, 1.0)",
        )
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            format!(
                "Discord upload outcome unknown; not retried: {}",
                e.without_url()
            )
        })?;
    let status = response.status();
    let bytes = read_response(response)
        .await
        .map_err(|e| format!("Discord upload outcome unknown; not retried: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Discord send failed: HTTP {status}: {}; not retried",
            diagnostic(&bytes, token)
        ));
    }
    parse_receipt(&bytes, &target.channel_id, &filenames)
}

async fn read_response(mut response: Response) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Discord response read failed: {}", e.without_url()))?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Discord response exceeded 1 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn diagnostic(bytes: &[u8], token: &str) -> String {
    String::from_utf8_lossy(bytes)
        .replace(token, "[REDACTED]")
        .chars()
        .filter(|c| !c.is_control())
        .take(1000)
        .collect()
}
fn parse_receipt(bytes: &[u8], channel: &str, filenames: &[String]) -> Result<Receipt, String> {
    let invalid = || {
        "Discord upload receipt is missing, invalid or does not match this request; outcome unknown, not retried".to_owned()
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let id = value["id"].as_str().ok_or_else(invalid)?;
    super::super::setup::validate_id(id).map_err(|_| invalid())?;
    let actual = value["attachments"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|v| {
            v["filename"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if value["channel_id"].as_str() != Some(channel) || actual != filenames {
        return Err(invalid());
    }
    Ok(Receipt {
        message_id: id.into(),
        channel_id: channel.into(),
        filenames: actual,
    })
}
