use std::fs;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Duration, Utc};

use crate::config::RemoteMcpConfig;

mod lifecycle;
mod model;
mod storage;
mod validation;

pub use lifecycle::{RestartHandoffLifecycleError, RestartHandoffRuntime};
use model::{Envelope, Payload};
pub use model::{HandoffProtector, RestartHandoffError, RestartProject, SystemProtector};
use storage::{claimed_path, system_protect, system_unprotect, write_atomic};
use validation::{canonical_project, gateway_fingerprint, validate_payload};

pub(super) const FORMAT_VERSION: u8 = 1;
pub(super) const PROTOCOL_VERSION: u8 = 10;
pub(super) const HANDOFF_TTL_SECONDS: i64 = 120;
pub(super) const MAX_HANDOFF_BYTES: u64 = 1_048_576;
pub(super) const MAX_PROJECTS: usize = 128;
pub const RESUME_ENV_NAME: &str = "CODEX_REMOTE_MCP_RESTART_RESUME";

impl HandoffProtector for SystemProtector {
    fn protect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
        system_protect(payload)
    }

    fn unprotect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
        system_unprotect(payload)
    }
}

pub fn write_restart_handoff_at(
    projects: &[RestartProject],
    config: &RemoteMcpConfig,
    path: &Path,
    protector: &dyn HandoffProtector,
    now: DateTime<Utc>,
) -> Result<bool, RestartHandoffError> {
    if projects.is_empty() {
        return Ok(false);
    }
    if projects.len() > MAX_PROJECTS {
        return Err(RestartHandoffError::TooLarge);
    }
    let payload = Payload {
        format_version: FORMAT_VERSION,
        protocol_version: PROTOCOL_VERSION,
        gateway_fingerprint: gateway_fingerprint(config),
        created_at: now,
        resume_until: now + Duration::seconds(HANDOFF_TTL_SECONDS),
        projects: projects.to_vec(),
    };
    let plaintext = serde_json::to_vec(&payload).map_err(|_| RestartHandoffError::Malformed)?;
    let ciphertext = protector.protect(&plaintext)?;
    let encoded = serde_json::to_vec(&Envelope {
        format_version: FORMAT_VERSION,
        ciphertext: STANDARD.encode(ciphertext),
    })
    .map_err(|_| RestartHandoffError::Malformed)?;
    if encoded.len() as u64 > MAX_HANDOFF_BYTES {
        return Err(RestartHandoffError::TooLarge);
    }
    write_atomic(path, &encoded)?;
    Ok(true)
}

pub fn claim_restart_handoff_at(
    config: &RemoteMcpConfig,
    path: &Path,
    protector: &dyn HandoffProtector,
    now: DateTime<Utc>,
    resume_requested: bool,
) -> Result<Vec<RestartProject>, RestartHandoffError> {
    if !resume_requested {
        return Ok(Vec::new());
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(RestartHandoffError::Malformed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    }
    let claimed = claimed_path(path);
    if let Err(error) = fs::rename(path, &claimed) {
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(Vec::new())
        } else {
            Err(error.into())
        };
    }
    let result = claim_file(config, &claimed, protector, now);
    let cleanup = fs::remove_file(&claimed);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Ok(projects), Ok(())) => Ok(projects),
    }
}

fn claim_file(
    config: &RemoteMcpConfig,
    path: &Path,
    protector: &dyn HandoffProtector,
    now: DateTime<Utc>,
) -> Result<Vec<RestartProject>, RestartHandoffError> {
    if fs::metadata(path)?.len() > MAX_HANDOFF_BYTES {
        return Err(RestartHandoffError::TooLarge);
    }
    let raw = fs::read(path)?;
    let envelope: Envelope =
        serde_json::from_slice(&raw).map_err(|_| RestartHandoffError::Malformed)?;
    if envelope.format_version != FORMAT_VERSION {
        return Err(RestartHandoffError::UnsupportedFormat);
    }
    let ciphertext = STANDARD
        .decode(envelope.ciphertext)
        .map_err(|_| RestartHandoffError::Malformed)?;
    let plaintext = protector.unprotect(&ciphertext)?;
    let payload: Payload =
        serde_json::from_slice(&plaintext).map_err(|_| RestartHandoffError::Malformed)?;
    validate_payload(&payload, config, now)?;
    payload
        .projects
        .into_iter()
        .map(canonical_project)
        .collect()
}
