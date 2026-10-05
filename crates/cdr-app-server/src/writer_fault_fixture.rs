//! Explicit, default-off test support for a real partial pipe write.
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use tokio::sync::Semaphore;

static ARMED: OnceLock<Mutex<Option<Arc<PrefixState>>>> = OnceLock::new();

/// Actual bytes accepted by the writer before the remainder is held.
#[derive(Clone, Debug)]
pub struct PrefixObservation {
    pub written: usize,
    pub frame_len: usize,
    pub wire_id: Value,
}

/// One scoped test fault. Dropping the handle releases a waiting writer as failed.
pub struct PrefixBarrier {
    state: Arc<PrefixState>,
}

pub(crate) struct PrefixState {
    target: String,
    prefix_bytes: usize,
    observation: Mutex<Option<PrefixObservation>>,
    entered: Semaphore,
    release: Semaphore,
}

/// Arm one turn/start prefix failure for this exact test target.
///
/// # Errors
/// Rejects an empty target, zero prefix, poisoned registry, or another armed fault.
pub fn arm_prefix(target: &str, prefix_bytes: usize) -> io::Result<PrefixBarrier> {
    if target.is_empty() || prefix_bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid prefix fixture",
        ));
    }
    let mut armed = ARMED
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| io::Error::other("prefix registry poisoned"))?;
    if armed.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "prefix fixture already armed",
        ));
    }
    let state = Arc::new(PrefixState {
        target: target.into(),
        prefix_bytes,
        observation: Mutex::new(None),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    *armed = Some(Arc::clone(&state));
    Ok(PrefixBarrier { state })
}

impl PrefixBarrier {
    /// Wait for evidence produced only after the real prefix write completes.
    ///
    /// # Errors
    /// Reports a closed barrier, poisoned observation, or missing evidence.
    pub async fn wait_prefix(&self) -> io::Result<PrefixObservation> {
        self.state
            .entered
            .acquire()
            .await
            .map_err(|_| io::Error::other("prefix barrier closed"))?
            .forget();
        self.state
            .observation
            .lock()
            .map_err(|_| io::Error::other("prefix observation poisoned"))?
            .clone()
            .ok_or_else(|| io::Error::other("prefix evidence missing"))
    }

    /// Release the original occurrence with an injected I/O error, never a resend.
    pub fn fail(&self) {
        self.state.release.add_permits(1);
    }
}

impl Drop for PrefixBarrier {
    fn drop(&mut self) {
        self.fail();
        if let Some(registry) = ARMED.get()
            && let Ok(mut armed) = registry.lock()
            && armed
                .as_ref()
                .is_some_and(|state| Arc::ptr_eq(state, &self.state))
        {
            *armed = None;
        }
    }
}

pub(crate) fn take_for(value: &Value) -> io::Result<Option<Arc<PrefixState>>> {
    let Some(registry) = ARMED.get() else {
        return Ok(None);
    };
    let mut armed = registry
        .lock()
        .map_err(|_| io::Error::other("prefix registry poisoned"))?;
    if armed.as_ref().is_some_and(|state| {
        value["method"] == "turn/start" && value["params"]["threadId"] == state.target
    }) {
        Ok(armed.take())
    } else {
        Ok(None)
    }
}

impl PrefixState {
    pub(crate) fn prefix_len(&self, frame_len: usize) -> io::Result<usize> {
        if self.prefix_bytes >= frame_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "prefix is not a partial frame",
            ));
        }
        Ok(self.prefix_bytes)
    }

    pub(crate) async fn after_prefix(
        &self,
        written: usize,
        frame_len: usize,
        wire_id: Value,
    ) -> io::Error {
        match self.observation.lock() {
            Ok(mut value) => {
                *value = Some(PrefixObservation {
                    written,
                    frame_len,
                    wire_id,
                });
            }
            Err(_) => return io::Error::other("prefix observation poisoned"),
        }
        self.entered.add_permits(1);
        match self.release.acquire().await {
            Ok(permit) => permit.forget(),
            Err(_) => return io::Error::other("prefix release closed"),
        }
        io::Error::other("injected failure after a real partial frame write")
    }
}
