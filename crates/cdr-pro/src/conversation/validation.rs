use sha2::{Digest, Sha256};
use url::Url;

pub(super) fn scope(value: &str) -> Result<(), String> {
    let suffix = value
        .strip_prefix("codex-pro-")
        .ok_or("The conversation scope is invalid.")?;
    if suffix.len() != 24
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("The conversation scope is invalid.".into());
    }
    Ok(())
}

pub(super) fn url(value: &str) -> Result<Url, String> {
    let error = "Only canonical HTTPS chatgpt.com conversation URLs are allowed.";
    let parsed = Url::parse(value).map_err(|_| error.to_owned())?;
    if parsed.scheme() != "https"
        || !matches!(parsed.host_str(), Some("chatgpt.com" | "www.chatgpt.com"))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !parsed.path().contains("/c/")
    {
        return Err(error.into());
    }
    Ok(parsed)
}

pub(super) fn same(left: &str, right: &str) -> Result<bool, String> {
    let left = url(left)?;
    let right = url(right)?;
    Ok(left.host_str().unwrap_or("").trim_start_matches("www.")
        == right.host_str().unwrap_or("").trim_start_matches("www.")
        && left.path().trim_end_matches('/') == right.path().trim_end_matches('/'))
}

pub(super) fn lease(token: &str) -> Result<(), String> {
    if token.len() < 24 {
        return Err("The conversation creation lease is invalid.".into());
    }
    Ok(())
}

pub(super) fn token() -> String {
    format!(
        "lease_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub(super) fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
pub(super) fn owns(hash: Option<&str>, token: &str) -> bool {
    let actual = self::hash(token);
    hash.is_some_and(|hash| {
        hash.len() == actual.len()
            && hash
                .bytes()
                .zip(actual.bytes())
                .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    })
}
