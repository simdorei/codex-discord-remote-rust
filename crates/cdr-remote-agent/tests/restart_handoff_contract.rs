use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use cdr_remote_agent::config::load_remote_mcp_config;
use cdr_remote_agent::restart_handoff::{
    HandoffProtector, RestartHandoffError, RestartProject, claim_restart_handoff_at,
    write_restart_handoff_at,
};
use chrono::{Duration, TimeZone, Utc};

struct XorProtector;

impl HandoffProtector for XorProtector {
    fn protect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
        Ok(payload.iter().map(|value| value ^ 0xA5).collect())
    }

    fn unprotect(&self, payload: &[u8]) -> Result<Vec<u8>, RestartHandoffError> {
        self.protect(payload)
    }
}

fn config() -> cdr_remote_agent::config::RemoteMcpConfig {
    load_remote_mcp_config(&HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            "wss://example.test/v12/bridge".into(),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-a".into()),
        (
            "CODEX_REMOTE_MCP_DEVICE_TOKEN".into(),
            "device-token-must-not-be-stored".into(),
        ),
    ]))
    .unwrap()
    .unwrap()
}

#[test]
fn encrypted_handoff_is_single_use_and_preserves_exact_binding() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = directory.path().join("project-root-must-not-be-plaintext");
    fs::create_dir(&project_root).unwrap();
    let handoff = directory.path().join("handoff.json");
    let now = Utc.with_ymd_and_hms(2026, 8, 31, 1, 2, 3).unwrap();
    let projects = vec![RestartProject {
        thread_id: "thread-scope-must-not-be-plaintext".into(),
        root: project_root.canonicalize().unwrap(),
        expires_at: now + Duration::minutes(10),
    }];

    assert!(write_restart_handoff_at(&projects, &config(), &handoff, &XorProtector, now).unwrap());
    let raw = fs::read(&handoff).unwrap();
    for secret in [
        projects[0].thread_id.as_bytes(),
        projects[0].root.to_string_lossy().as_bytes(),
        b"device-token-must-not-be-stored",
    ] {
        assert!(!raw.windows(secret.len()).any(|window| window == secret));
    }

    let restored = claim_restart_handoff_at(
        &config(),
        &handoff,
        &XorProtector,
        now + Duration::seconds(1),
        true,
    )
    .unwrap();
    assert_eq!(restored, projects);
    assert!(!handoff.exists());
    assert!(
        claim_restart_handoff_at(&config(), &handoff, &XorProtector, now, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn normal_start_leaves_handoff_unclaimed() {
    let (directory, handoff, now, projects) = fixture();
    write_restart_handoff_at(&projects, &config(), &handoff, &XorProtector, now).unwrap();

    let restored =
        claim_restart_handoff_at(&config(), &handoff, &XorProtector, now, false).unwrap();

    assert!(restored.is_empty());
    assert!(handoff.is_file());
    drop(directory);
}

#[test]
fn expired_handoff_is_consumed_without_restoring() {
    let (directory, handoff, now, projects) = fixture();
    write_restart_handoff_at(&projects, &config(), &handoff, &XorProtector, now).unwrap();

    let error = claim_restart_handoff_at(
        &config(),
        &handoff,
        &XorProtector,
        now + Duration::minutes(3),
        true,
    )
    .unwrap_err();

    assert!(matches!(error, RestartHandoffError::Expired));
    assert!(!handoff.exists());
    drop(directory);
}

fn fixture() -> (
    tempfile::TempDir,
    PathBuf,
    chrono::DateTime<Utc>,
    Vec<RestartProject>,
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    fs::create_dir(&root).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 31, 1, 2, 3).unwrap();
    let projects = vec![RestartProject {
        thread_id: "thread-a".into(),
        root: root.canonicalize().unwrap(),
        expires_at: now + Duration::minutes(10),
    }];
    let handoff = directory.path().join("handoff.json");
    (directory, handoff, now, projects)
}
