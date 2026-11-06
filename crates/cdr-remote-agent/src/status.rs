use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const EXTERNAL_MCP_STATUS_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_EXTERNAL_MCP_STATUS_STALE_MS: i64 = 6_000;

#[must_use]
pub fn bridge_liveness_stale_ms() -> i64 {
    std::env::var("CDR_REMOTE_MCP_LIVENESS_STALE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|&v| (200..=60_000).contains(&v))
        .unwrap_or(DEFAULT_EXTERNAL_MCP_STATUS_STALE_MS)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExternalMcpBuildInfo {
    pub schema_version: u32,
    pub package_version: String,
    pub source_commit: Option<String>,
    pub source_tree: Option<String>,
    pub source_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExternalMcpStatusRecord {
    pub schema_version: u32,
    pub component: String,
    pub install_root: String,
    pub process_identity: String,
    pub pid: u32,
    pub executable_path: String,
    pub artifact_sha256: String,
    pub build_info: ExternalMcpBuildInfo,
    pub updated_at_ms: i64,
    pub heartbeat_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_verified_at_ms: Option<i64>,
    pub device_id: String,
    pub bridge_fingerprint: String,
    pub hello_ack: bool,
    pub ready: bool,
    pub draining: bool,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub struct ExternalMcpStatusPublisher {
    status_path: PathBuf,
    install_root: String,
    process_identity: String,
    pid: u32,
    executable_path: String,
    artifact_sha256: String,
    build_info: ExternalMcpBuildInfo,
    device_id: String,
    bridge_fingerprint: String,
}

impl ExternalMcpStatusPublisher {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        status_path: PathBuf,
        install_root: &Path,
        process_identity: String,
        pid: u32,
        executable_path: &Path,
        artifact_sha256: String,
        build_info: ExternalMcpBuildInfo,
        device_id: String,
        bridge_fingerprint: String,
    ) -> Self {
        Self {
            status_path,
            install_root: normalize_install_root(install_root),
            process_identity,
            pid,
            executable_path: executable_path.display().to_string(),
            artifact_sha256,
            build_info,
            device_id,
            bridge_fingerprint,
        }
    }

    pub fn publish(
        &self,
        hello_ack: bool,
        ready: bool,
        draining: bool,
        generation: u64,
    ) -> std::io::Result<ExternalMcpStatusRecord> {
        let now_ms = Utc::now().timestamp_millis();
        let connection_verified_at_ms = if ready && hello_ack && !draining && generation > 0 {
            Some(now_ms)
        } else {
            None
        };
        self.publish_with_timestamps(
            hello_ack,
            ready,
            draining,
            generation,
            now_ms,
            now_ms,
            connection_verified_at_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_with_timestamps(
        &self,
        hello_ack: bool,
        ready: bool,
        draining: bool,
        generation: u64,
        updated_at_ms: i64,
        heartbeat_at_ms: i64,
        connection_verified_at_ms: Option<i64>,
    ) -> std::io::Result<ExternalMcpStatusRecord> {
        let record = ExternalMcpStatusRecord {
            schema_version: EXTERNAL_MCP_STATUS_SCHEMA_VERSION,
            component: "mcp".into(),
            install_root: self.install_root.clone(),
            process_identity: self.process_identity.clone(),
            pid: self.pid,
            executable_path: self.executable_path.clone(),
            artifact_sha256: self.artifact_sha256.clone(),
            build_info: self.build_info.clone(),
            updated_at_ms,
            heartbeat_at_ms,
            connection_verified_at_ms,
            device_id: self.device_id.clone(),
            bridge_fingerprint: self.bridge_fingerprint.clone(),
            hello_ack: hello_ack && !draining,
            ready: ready && hello_ack && !draining,
            draining,
            generation,
        };
        write_external_mcp_status_atomic(&self.status_path, &record)?;
        Ok(record)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RemoteAgentStatus {
    inner: Arc<StatusInner>,
}

#[derive(Debug, Default)]
struct StatusInner {
    connected: AtomicBool,
    draining: AtomicBool,
    shutting_down: AtomicBool,
    generation: AtomicU64,
    connected_generation: AtomicU64,
    last_bridge_rx_at_ms: AtomicI64,
    publisher: Mutex<Option<ExternalMcpStatusPublisher>>,
}

impl RemoteAgentStatus {
    #[must_use]
    pub fn with_external_publisher(publisher: ExternalMcpStatusPublisher) -> Self {
        let status = Self::default();
        status.attach_external_publisher(publisher);
        status
    }

    pub fn attach_external_publisher(&self, publisher: ExternalMcpStatusPublisher) {
        if let Ok(mut guard) = self.inner.publisher.lock() {
            *guard = Some(publisher);
        }
        self.publish_snapshot();
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        let now_ms = Utc::now().timestamp_millis();
        self.inner.connected.load(Ordering::Acquire)
            && !self.is_draining()
            && !self.is_shutting_down()
            && self.has_fresh_bridge_liveness(now_ms)
    }

    #[must_use]
    pub fn has_fresh_bridge_liveness(&self, now_ms: i64) -> bool {
        let generation = self.generation();
        let connected_gen = self.inner.connected_generation.load(Ordering::Acquire);
        let last_rx = self.inner.last_bridge_rx_at_ms.load(Ordering::Acquire);
        let stale_ms = bridge_liveness_stale_ms();
        generation > 0
            && connected_gen == generation
            && last_rx > 0
            && last_rx <= now_ms + 2_000
            && now_ms.saturating_sub(last_rx) <= stale_ms
    }

    #[must_use]
    pub fn last_bridge_rx_at_ms(&self) -> i64 {
        self.inner.last_bridge_rx_at_ms.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_draining(&self) -> bool {
        self.inner.draining.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_shutting_down(&self) -> bool {
        self.inner.shutting_down.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    pub fn mark_draining(&self) {
        self.inner.draining.store(true, Ordering::Release);
        self.inner.connected.store(false, Ordering::Release);
        self.inner.connected_generation.store(0, Ordering::Release);
        self.inner.last_bridge_rx_at_ms.store(0, Ordering::Release);
        self.publish_snapshot();
    }

    pub fn mark_shutting_down(&self) {
        self.inner.shutting_down.store(true, Ordering::Release);
        self.inner.connected.store(false, Ordering::Release);
        self.inner.connected_generation.store(0, Ordering::Release);
        self.inner.last_bridge_rx_at_ms.store(0, Ordering::Release);
        self.publish_snapshot();
    }

    pub fn record_heartbeat(&self) {
        self.publish_snapshot();
    }

    pub fn record_bridge_inbound(&self, generation: u64) {
        if generation > 0
            && !self.is_draining()
            && !self.is_shutting_down()
            && self.inner.connected.load(Ordering::Acquire)
            && self.inner.generation.load(Ordering::Acquire) == generation
        {
            let now_ms = Utc::now().timestamp_millis();
            self.inner
                .connected_generation
                .store(generation, Ordering::Release);
            self.inner
                .last_bridge_rx_at_ms
                .store(now_ms, Ordering::Release);
            self.publish_snapshot();
        }
    }

    pub(crate) fn set_connected(&self, generation: u64) {
        let now_ms = Utc::now().timestamp_millis();
        self.inner.generation.store(generation, Ordering::Release);
        if !self.is_draining() && !self.is_shutting_down() {
            self.inner
                .connected_generation
                .store(generation, Ordering::Release);
            self.inner
                .last_bridge_rx_at_ms
                .store(now_ms, Ordering::Release);
            self.inner.connected.store(true, Ordering::Release);
        }
        self.publish_snapshot();
    }

    pub(crate) fn set_disconnected(&self) {
        self.inner.connected.store(false, Ordering::Release);
        self.inner.connected_generation.store(0, Ordering::Release);
        self.inner.last_bridge_rx_at_ms.store(0, Ordering::Release);
        self.publish_snapshot();
    }

    fn publish_snapshot(&self) {
        if let Ok(guard) = self.inner.publisher.lock()
            && let Some(publisher) = guard.as_ref()
        {
            let now_ms = Utc::now().timestamp_millis();
            let draining = self.is_draining();
            let shutting_down = self.is_shutting_down();
            let generation = self.generation();
            let conn_gen = self.inner.connected_generation.load(Ordering::Acquire);
            let last_rx = self.inner.last_bridge_rx_at_ms.load(Ordering::Acquire);
            let raw_connected =
                self.inner.connected.load(Ordering::Acquire) && !draining && !shutting_down;
            let fresh_liveness = raw_connected && self.has_fresh_bridge_liveness(now_ms);
            let connection_verified_at_ms =
                if raw_connected && generation > 0 && conn_gen == generation && last_rx > 0 {
                    Some(last_rx)
                } else {
                    None
                };
            let updated_at_ms = connection_verified_at_ms.unwrap_or(now_ms);
            let _ = publisher.publish_with_timestamps(
                fresh_liveness,
                fresh_liveness,
                draining,
                generation,
                updated_at_ms,
                now_ms,
                connection_verified_at_ms,
            );
        }
    }
}

#[must_use]
pub fn normalize_install_root(path: &Path) -> String {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let raw = resolved.to_string_lossy();
    let trimmed_prefix = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    let mut normalized = trimmed_prefix.replace('/', r"\");
    while normalized.len() > 3 && normalized.ends_with('\\') {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.make_ascii_lowercase();
    }
    normalized
}

pub fn sha256_file_hex(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn write_external_mcp_status_atomic(
    destination: &Path,
    record: &ExternalMcpStatusRecord,
) -> std::io::Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_vec_pretty(record)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let file_name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let temp_path =
        destination.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4().simple()));
    {
        let mut temp_file = File::create(&temp_path)?;
        temp_file.write_all(&payload)?;
        temp_file.flush()?;
        temp_file.sync_all()?;
    }
    #[cfg(windows)]
    let replace_result = cdr_windows_native::atomic_replace(&temp_path, destination)
        .map_err(|error| std::io::Error::other(error.to_string()));
    #[cfg(not(windows))]
    let replace_result = fs::rename(&temp_path, destination);

    if let Err(error) = replace_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}
