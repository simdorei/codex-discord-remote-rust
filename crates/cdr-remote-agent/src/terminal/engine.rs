use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cdr_core::deadline::RequestBudget;
use cdr_remote_protocol::identifiers::TerminalId;
use cdr_remote_protocol::output::TerminalOutput;
use cdr_remote_protocol::request::TerminalRequest;
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

use super::TerminalError;
use super::shell::inherited_environment;

mod execution;

const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct TerminalExecutionEngine {
    root: PathBuf,
    session_id: String,
    state: Mutex<EngineState>,
}

struct EngineState {
    closed: bool,
    terminals: HashMap<String, Arc<TerminalEntry>>,
}

struct TerminalEntry {
    state: Mutex<TerminalState>,
}

struct TerminalState {
    cwd: PathBuf,
    environment: HashMap<String, String>,
    active: Option<Arc<ExecutionTicket>>,
}

struct ExecutionTicket {
    cancel: watch::Sender<bool>,
    done: watch::Sender<bool>,
}

impl TerminalExecutionEngine {
    pub fn new(root: &Path, session_id: &str) -> Result<Self, TerminalError> {
        if session_id.is_empty() {
            return Err(TerminalError::EmptySession);
        }
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(TerminalError::InvalidRoot);
        }
        Ok(Self {
            root,
            session_id: session_id.to_owned(),
            state: Mutex::new(EngineState {
                closed: false,
                terminals: HashMap::new(),
            }),
        })
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub async fn execute(
        &self,
        request: &TerminalRequest,
        cancelled: watch::Receiver<bool>,
        budget: &RequestBudget,
    ) -> Result<TerminalOutput, TerminalError> {
        if *cancelled.borrow() {
            return Err(TerminalError::Cancelled);
        }
        let TerminalRequest::TerminalExec {
            terminal_id,
            shell,
            command,
            cwd,
            environment,
            timeout_seconds,
            cancel_previous,
        } = request
        else {
            return Err(TerminalError::UnsupportedRequest);
        };
        let (terminal_id, entry, ticket) = self
            .reserve(terminal_id.as_ref(), *cancel_previous, cancelled.clone())
            .await?;
        let result = self
            .execute_reserved(
                &terminal_id,
                &entry,
                &ticket,
                *shell,
                command,
                cwd.as_deref(),
                environment,
                *timeout_seconds,
                cancelled,
                budget,
            )
            .await;
        self.release(&entry, &ticket).await;
        result
    }

    pub async fn close(&self) -> Result<(), TerminalError> {
        let entries = {
            let mut state = self.state.lock().await;
            if state.closed {
                return Ok(());
            }
            state.closed = true;
            state
                .terminals
                .drain()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>()
        };
        let mut active = Vec::new();
        for entry in entries {
            if let Some(ticket) = entry.state.lock().await.active.clone() {
                ticket.cancel.send_replace(true);
                active.push(ticket.done.subscribe());
            }
        }
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        for done in active {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero()
                || tokio::time::timeout(remaining, wait_until_true(done))
                    .await
                    .is_err()
            {
                return Err(TerminalError::CloseTimeout);
            }
        }
        Ok(())
    }

    async fn reserve(
        &self,
        requested_id: Option<&TerminalId>,
        cancel_previous: bool,
        cancelled: watch::Receiver<bool>,
    ) -> Result<(TerminalId, Arc<TerminalEntry>, Arc<ExecutionTicket>), TerminalError> {
        loop {
            if *cancelled.borrow() {
                return Err(TerminalError::Cancelled);
            }
            let mut engine = self.state.lock().await;
            if engine.closed {
                return Err(TerminalError::Closed);
            }
            let (terminal_id, entry) = terminal(&mut engine, requested_id, &self.root)?;
            let mut state = entry.state.lock().await;
            if state.active.is_none() {
                let (cancel, _) = watch::channel(false);
                let (done, _) = watch::channel(false);
                let ticket = Arc::new(ExecutionTicket { cancel, done });
                state.active = Some(Arc::clone(&ticket));
                drop(state);
                drop(engine);
                return Ok((terminal_id, entry, ticket));
            }
            let previous = state.active.clone().expect("active terminal ticket");
            drop(state);
            drop(engine);
            if !cancel_previous {
                return Err(TerminalError::ActiveCommand);
            }
            previous.cancel.send_replace(true);
            tokio::select! {
                () = wait_until_true(previous.done.subscribe()) => {}
                () = wait_until_true(cancelled.clone()) => return Err(TerminalError::Cancelled),
            }
        }
    }

    async fn release(&self, entry: &TerminalEntry, ticket: &Arc<ExecutionTicket>) {
        let mut state = entry.state.lock().await;
        if state
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, ticket))
        {
            state.active = None;
        }
        ticket.done.send_replace(true);
    }
}

fn terminal(
    engine: &mut EngineState,
    requested_id: Option<&TerminalId>,
    root: &Path,
) -> Result<(TerminalId, Arc<TerminalEntry>), TerminalError> {
    if let Some(id) = requested_id {
        let entry = engine
            .terminals
            .get(&id.0)
            .cloned()
            .ok_or(TerminalError::ForeignTerminal)?;
        return Ok((id.clone(), entry));
    }
    loop {
        let id = generated_id("term_");
        if !engine.terminals.contains_key(&id) {
            let entry = Arc::new(TerminalEntry {
                state: Mutex::new(TerminalState {
                    cwd: root.to_path_buf(),
                    environment: inherited_environment(),
                    active: None,
                }),
            });
            engine.terminals.insert(id.clone(), Arc::clone(&entry));
            return Ok((TerminalId(id), entry));
        }
    }
}

fn generated_id(prefix: &str) -> String {
    let random = Uuid::new_v4().simple().to_string();
    format!("{prefix}{}", &random[..16])
}

async fn wait_until_true(mut receiver: watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
