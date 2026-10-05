//! Native read lifetime only: not user consent or permission to start a request.
use super::{ResidentAppServer, admission::ResidentAdmission};
use crate::{AppServerError, requests::AppRequest};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::time::Instant;

mod collector;
const BUDGET: Duration = Duration::from_secs(10);
const REQUEST_LIMIT: Duration = Duration::from_secs(2);

struct ReadLifetime {
    server: Arc<ResidentAppServer>,
    admission: ResidentAdmission,
    thread: String,
    owners: BTreeSet<String>,
    deadline: Instant,
    request_limit: Duration,
}

/// A completed bounded observation tied to one real resident/client lifetime.
///
/// No Clone/Deserialize implementation or public constructor exists. The caller
/// must retain its target/control permits and acquire separate authenticated
/// release consent; this object never changes a policy or starts a turn.
///
/// Caller JSON cannot fabricate the native lifetime:
/// ```compile_fail
/// let _: cdr_app_server::NativeRecoveryObservation =
///     serde_json::from_value(serde_json::json!({})).unwrap();
/// ```
pub struct NativeRecoveryObservation {
    lifetime: ReadLifetime,
    observation: Value,
}

impl ResidentAppServer {
    /// Read only through the normal resident dispatch/pending/writer guards.
    pub async fn observe_recovery_prerequisites(
        self: &Arc<Self>,
        thread: &str,
        owners: &[String],
        request_limit: Duration,
    ) -> Result<NativeRecoveryObservation, AppServerError> {
        let wanted = owners.iter().cloned().collect::<BTreeSet<_>>();
        if thread.trim().is_empty()
            || thread.len() > 512
            || owners.is_empty()
            || owners.len() > 128
            || wanted.len() != owners.len()
            || owners
                .iter()
                .any(|id| id.trim().is_empty() || id.len() > 512)
            || request_limit.is_zero()
        {
            return Err(invalid(
                "bounded exact target and original owners are required",
            ));
        }
        let lifetime = ReadLifetime {
            server: Arc::clone(self),
            admission: self.state.admit_request(None)?,
            thread: thread.into(),
            owners: wanted,
            deadline: Instant::now() + BUDGET,
            request_limit: request_limit.min(REQUEST_LIMIT),
        };
        lifetime.current(|| ())?;
        let observation = tokio::time::timeout_at(lifetime.deadline, collector::collect(&lifetime))
            .await
            .map_err(|_| expired())??;
        lifetime.current(|| ())?;
        Ok(NativeRecoveryObservation {
            lifetime,
            observation,
        })
    }
}

impl NativeRecoveryObservation {
    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.lifetime.thread
    }
    #[must_use]
    pub fn resident_id(&self) -> &str {
        self.lifetime.server.instance_id()
    }
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.lifetime.admission.generation
    }
    #[must_use]
    pub fn observation(&self) -> &Value {
        &self.observation
    }

    pub fn check_current(&self) -> Result<(), AppServerError> {
        self.lifetime.current(|| ())
    }

    /// Consume once under the same resident and client-lifecycle locks.
    /// Only a pre-acquired transaction's bounded final commit belongs here:
    /// no DB open/BEGIN/wait, parsing, RPC, reentrant resident call, or permit Drop.
    /// A durable record must still be revalidated by its actual execution writer
    /// after any subsequent invalidation. This is not release authorization.
    pub fn with_current_connection<R>(
        self,
        publish: impl FnOnce() -> R,
    ) -> Result<R, AppServerError> {
        self.lifetime.current(publish)
    }
}

impl ReadLifetime {
    fn current<R>(&self, action: impl FnOnce() -> R) -> Result<R, AppServerError> {
        self.server.state.with_recovery_current(
            &self.admission.client,
            self.admission.generation,
            || {
                if Instant::now() >= self.deadline {
                    Err(expired())
                } else {
                    Ok(action())
                }
            },
        )?
    }

    async fn request(&self, method: &'static str, params: Value) -> Result<Value, AppServerError> {
        self.current(|| ())?;
        let timeout = self
            .request_limit
            .min(self.deadline.saturating_duration_since(Instant::now()));
        if timeout.is_zero() {
            return Err(expired());
        }
        let result = self
            .server
            .request_for_recovery_observation(
                &self.admission,
                AppRequest {
                    method,
                    params,
                    timeout,
                },
            )
            .await?;
        self.current(|| ())?;
        Ok(result)
    }
}

fn invalid(message: &str) -> AppServerError {
    AppServerError::InvalidReply {
        message: message.into(),
    }
}

fn expired() -> AppServerError {
    AppServerError::Timeout {
        method: "recovery/read-only-observation".into(),
        timeout_ms: BUDGET.as_millis(),
    }
}
