use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use cdr_remote_agent::config::load_remote_mcp_config;
use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_agent::restart_handoff::{
    HandoffProtector, RestartHandoffError, RestartHandoffRuntime,
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

#[tokio::test]
async fn lifecycle_prepares_and_restores_dispatcher_bindings_once() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let path = directory.path().join("handoff.json");
    let now = Utc.with_ymd_and_hms(2026, 8, 31, 1, 2, 3).unwrap();
    let config = config();
    let source = LocalProjectDispatcher::new();
    source
        .upsert("thread-a", &project, now + Duration::minutes(10))
        .await
        .unwrap();
    let prepare = RestartHandoffRuntime::new(&path, Arc::new(XorProtector), false);

    assert!(prepare.prepare(&source, &config, now).await.unwrap());

    let replacement = LocalProjectDispatcher::new();
    let resume = RestartHandoffRuntime::new(&path, Arc::new(XorProtector), true);
    assert_eq!(
        resume
            .restore(&replacement, &config, now + Duration::seconds(1))
            .await
            .unwrap(),
        1
    );
    assert_eq!(replacement.restart_projects(now).await.len(), 1);
    assert!(!path.exists());
}

fn config() -> cdr_remote_agent::config::RemoteMcpConfig {
    load_remote_mcp_config(&HashMap::from([
        ("CODEX_REMOTE_MCP_ENABLED".into(), "1".into()),
        (
            "CODEX_REMOTE_MCP_BRIDGE_URL".into(),
            "wss://example.test/v12/bridge".into(),
        ),
        ("CODEX_REMOTE_MCP_DEVICE_ID".into(), "device-a".into()),
        ("CODEX_REMOTE_MCP_DEVICE_TOKEN".into(), "secret".into()),
    ]))
    .unwrap()
    .unwrap()
}
