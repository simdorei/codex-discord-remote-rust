use base64::Engine as _;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use url::Url;
use uuid::Uuid;

use super::OAuthProviderError;
use crate::scopes::ALL;

pub(super) fn new_secret(parts: usize) -> String {
    (0..parts)
        .map(|_| Uuid::new_v4().simple().to_string())
        .collect()
}

pub(super) fn constant_time_matches(candidate: &str, expected: &str) -> bool {
    let candidate = Sha256::digest(candidate.as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    bool::from(candidate.ct_eq(&expected))
}

pub(super) fn verify_pkce(verifier: &str, challenge: &str) -> bool {
    let actual = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    constant_time_matches(&actual, challenge)
}

pub(super) fn validate_scopes(scopes: &[String]) -> Result<(), OAuthProviderError> {
    if scopes.iter().all(|scope| ALL.contains(&scope.as_str())) {
        Ok(())
    } else {
        Err(OAuthProviderError::InvalidScope)
    }
}

pub(super) fn endpoint(base: &Url, suffix: &str) -> Result<Url, OAuthProviderError> {
    Ok(format!("{}{suffix}", base.as_str().trim_end_matches('/')).parse()?)
}

pub(super) fn now() -> Result<i64, OAuthProviderError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| OAuthProviderError::Configuration("system clock is invalid"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| OAuthProviderError::Configuration("system clock is invalid"))
}
