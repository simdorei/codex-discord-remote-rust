use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use cdr_core::canonical_json::to_canonical_json;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::request::{CoreRequest, ProjectOperation};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::computer::{ComputerAccessMode, ComputerController};
use crate::files::{ProjectFileAccess, RemoteFileError};
use crate::terminal::{TerminalExecutionEngine, TerminalWindowManager};

const MAX_CACHED_RESULTS: usize = 256;

#[derive(Default)]
pub struct DispatchState {
    pub connection_generation: Option<u64>,
    pub projects: HashMap<String, Arc<ActiveProject>>,
}

pub struct ActiveProject {
    pub access: ProjectFileAccess,
    pub expires_at: DateTime<Utc>,
    pub session: Mutex<Option<Arc<SessionActivation>>>,
    pub mutations: Mutex<MutationCache>,
}

impl ActiveProject {
    pub fn open(root: &Path, expires_at: DateTime<Utc>) -> Result<Self, RemoteFileError> {
        Ok(Self {
            access: ProjectFileAccess::open(root)?,
            expires_at,
            session: Mutex::new(None),
            mutations: Mutex::new(MutationCache::default()),
        })
    }
}

pub struct SessionActivation {
    pub connection_generation: Option<u64>,
    pub session_generation: u64,
    pub session_id: String,
    pub computer_mode: ComputerAccessMode,
    pub computer: ComputerController,
    pub terminals: TerminalExecutionEngine,
    pub terminal_windows: TerminalWindowManager,
}

#[derive(Default)]
pub struct MutationCache {
    values: HashMap<(String, Option<String>, String), CachedResult>,
    order: VecDeque<(String, Option<String>, String)>,
}

struct CachedResult {
    fingerprint: String,
    result: BridgeResult,
}

impl MutationCache {
    pub fn lookup(&mut self, command: &GatewayCommand) -> Option<BridgeResult> {
        let key = cache_key(command);
        let cached = self.values.get(&key)?;
        if cached.fingerprint == fingerprint(command) {
            self.order.retain(|item| item != &key);
            self.order.push_back(key);
            Some(cached.result.clone())
        } else {
            Some(conflict(command))
        }
    }

    pub fn remember(&mut self, command: &GatewayCommand, result: &BridgeResult) {
        if matches!(result, BridgeResult::OperationError { .. }) {
            return;
        }
        let key = cache_key(command);
        self.order.retain(|item| item != &key);
        self.order.push_back(key.clone());
        self.values.insert(
            key,
            CachedResult {
                fingerprint: fingerprint(command),
                result: result.clone(),
            },
        );
        while self.order.len() > MAX_CACHED_RESULTS {
            if let Some(oldest) = self.order.pop_front() {
                self.values.remove(&oldest);
            }
        }
    }
}

pub fn session_activation(command: &GatewayCommand) -> Option<(&str, u64, ComputerAccessMode)> {
    match command {
        GatewayCommand::ProjectSession {
            computer_session_id,
            computer_session_generation,
            ..
        } => Some((
            computer_session_id,
            *computer_session_generation,
            ComputerAccessMode::Project,
        )),
        GatewayCommand::DeviceSession {
            computer_session_id,
            computer_session_generation,
            ..
        } => Some((
            computer_session_id,
            *computer_session_generation,
            ComputerAccessMode::Device,
        )),
        _ => None,
    }
}

pub fn is_mutation(command: &GatewayCommand) -> bool {
    match command {
        GatewayCommand::WriteFile { .. } => true,
        GatewayCommand::ProjectOperation { operation, .. } => !matches!(
            operation,
            ProjectOperation::Core(
                CoreRequest::ProjectRules
                    | CoreRequest::ProjectStatus
                    | CoreRequest::CodeSearch { .. }
                    | CoreRequest::CommandList
                    | CoreRequest::RepoStatus
                    | CoreRequest::RepoDiff
                    | CoreRequest::ListImages
                    | CoreRequest::RetrieveImage { .. }
                    | CoreRequest::CheckpointList
                    | CoreRequest::CheckpointShow { .. }
            ) | ProjectOperation::Terminal(
                cdr_remote_protocol::request::TerminalRequest::TerminalExec { .. }
            )
        ),
        _ => false,
    }
}

fn cache_key(command: &GatewayCommand) -> (String, Option<String>, String) {
    let meta = super::error::command_meta(command);
    (
        meta.thread_id.to_owned(),
        meta.computer_session_id.map(str::to_owned),
        meta.request_id.to_owned(),
    )
}

fn fingerprint(command: &GatewayCommand) -> String {
    let canonical = to_canonical_json(command, &["deadline_at", "request_id"])
        .expect("gateway command always serializes");
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

fn conflict(command: &GatewayCommand) -> BridgeResult {
    BridgeResult::OperationError {
        request_id: super::error::command_meta(command).request_id.to_owned(),
        error_code: "request_id_conflict".into(),
        message: "The request ID was reused with different command content.".into(),
    }
}
