use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::model::{Payload, RestartHandoffError, RestartProject};
use super::{FORMAT_VERSION, MAX_PROJECTS, PROTOCOL_VERSION};
use crate::config::RemoteMcpConfig;

pub(super) fn validate_payload(
    payload: &Payload,
    config: &RemoteMcpConfig,
    now: DateTime<Utc>,
) -> Result<(), RestartHandoffError> {
    if payload.format_version != FORMAT_VERSION {
        return Err(RestartHandoffError::UnsupportedFormat);
    }
    if payload.protocol_version != PROTOCOL_VERSION {
        return Err(RestartHandoffError::ProtocolMismatch);
    }
    if !constant_time_eq(&payload.gateway_fingerprint, &gateway_fingerprint(config)) {
        return Err(RestartHandoffError::GatewayMismatch);
    }
    if payload.resume_until <= now {
        return Err(RestartHandoffError::Expired);
    }
    if payload.projects.is_empty() {
        return Err(RestartHandoffError::NoProjects);
    }
    if payload.projects.len() > MAX_PROJECTS {
        return Err(RestartHandoffError::TooLarge);
    }
    if payload.projects.iter().any(|item| item.expires_at <= now) {
        return Err(RestartHandoffError::ProjectExpired);
    }
    Ok(())
}

pub(super) fn canonical_project(
    mut project: RestartProject,
) -> Result<RestartProject, RestartHandoffError> {
    project.root = project
        .root
        .canonicalize()
        .map_err(|_| RestartHandoffError::ProjectRootUnavailable)?;
    if !project.root.is_dir() {
        return Err(RestartHandoffError::ProjectRootUnavailable);
    }
    Ok(project)
}

pub(super) fn gateway_fingerprint(config: &RemoteMcpConfig) -> String {
    let value = format!("{}\0{}", config.bridge_url, config.device_id);
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |different, (a, b)| different | (a ^ b))
            == 0
}
