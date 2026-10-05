use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cdr_app_server::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId, ServerRequest,
};
use cdr_runtime::dead_generation_recovery::RuntimeDeadGenerationFence;
use cdr_store::ingress::stop::{StopScope, control};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Timing {
    Before,
    After,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Stop,
    Recovery,
    StopThenRecovery,
    RecoveryRollback,
}

#[derive(Clone, Copy, Debug)]
pub struct Race {
    pub timing: Timing,
    pub action: Action,
    pub fail_finish: bool,
}

pub struct OrderedFence {
    inner: RuntimeDeadGenerationFence,
    db: PathBuf,
    race: Race,
    fired: AtomicBool,
    pub original_stop: Mutex<Option<control::StopControl>>,
    pub original_authority: Mutex<Option<Value>>,
}

impl OrderedFence {
    pub fn new(inner: RuntimeDeadGenerationFence, db: PathBuf, race: Race) -> Self {
        Self {
            inner,
            db,
            race,
            fired: AtomicBool::new(false),
            original_stop: Mutex::new(None),
            original_authority: Mutex::new(None),
        }
    }

    fn accept_stop(&self, owner: (&str, u64)) {
        let stop = control::accept_running(
            &self.db, StopScope { target:"thread-b", channel:42, owner:3 },
            &json!({"target":"thread-b","route":"Explicit","command":{"Stop":{"reference":"thread-b"}}}),
            None, (owner.0, i64::try_from(owner.1).unwrap()), || Ok(()),
        ).unwrap().expect("real Running-stop receipt");
        *self.original_stop.lock().unwrap() = Some(stop);
    }

    fn cancel_recovery(&self) {
        let result = cdr_store::queue::cancel_for_recovery(&self.db, "thread-b", 42, 3, 2.0);
        if self.race.action == Action::RecoveryRollback {
            assert!(
                result.is_err(),
                "injected transaction failure must be reported"
            );
        } else {
            assert_eq!(result.unwrap().jobs, ["original-a"]);
        }
    }

    fn intervene(&self, owner: (&str, u64)) {
        match self.race.action {
            Action::Stop => self.accept_stop(owner),
            Action::Recovery => self.cancel_recovery(),
            Action::StopThenRecovery => {
                self.accept_stop(owner);
                let before = cdr_store::execution_hold::reason(&self.db, "original-a").unwrap();
                self.cancel_recovery();
                assert_eq!(
                    cdr_store::execution_hold::reason(&self.db, "original-a").unwrap(),
                    before
                );
            }
            Action::RecoveryRollback => {
                rusqlite::Connection::open(&self.db).unwrap().execute_batch(
                    "CREATE TRIGGER reject_order_recovery BEFORE DELETE ON codex_turn_queue
                     WHEN OLD.job_id='original-a' BEGIN SELECT RAISE(ABORT,'injected recovery rollback'); END;"
                ).unwrap();
                self.cancel_recovery();
            }
        }
        if self.race.fail_finish {
            rusqlite::Connection::open(&self.db).unwrap().execute_batch(
                "CREATE TRIGGER reject_order_finish BEFORE UPDATE OF phase ON cdr_server_responses
                 WHEN NEW.target_thread_id='thread-b' AND NEW.phase='flushed'
                 BEGIN SELECT RAISE(ABORT,'injected response completion failure'); END;"
            ).unwrap();
        }
    }
}

impl DeadGenerationFence for OrderedFence {
    fn persist(&self, work: &DeadGenerationWork) -> Result<(), AppServerError> {
        self.inner.persist(work)
    }

    fn check_request(
        &self,
        generation: u64,
        method: &str,
        params: &Value,
    ) -> Result<(), AppServerError> {
        self.inner.check_request(generation, method, params)
    }

    fn begin_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        method: &str,
        params: &Value,
        scoped: bool,
    ) -> Result<bool, AppServerError> {
        self.inner
            .begin_mutation(owner, request, method, params, scoped)
    }

    fn finish_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        outcome: &str,
    ) -> Result<(), AppServerError> {
        self.inner.finish_mutation(owner, request, outcome)
    }

    fn response_authority(
        &self,
        owner: (&str, u64),
        request: &ServerRequest,
    ) -> Result<Option<Value>, AppServerError> {
        let authority = self.inner.response_authority(owner, request)?;
        if request.params["threadId"] == "thread-b" {
            (*self.original_authority.lock().unwrap()).clone_from(&authority);
        }
        Ok(authority)
    }

    fn begin_response(
        &self,
        owner: (&str, u64),
        request: &ServerRequest,
        authority: &Value,
        payload: &Value,
    ) -> Result<(), AppServerError> {
        if request.params["threadId"] != "thread-b" || self.fired.swap(true, Ordering::AcqRel) {
            return self
                .inner
                .begin_response(owner, request, authority, payload);
        }
        if self.race.timing == Timing::Before {
            self.intervene(owner);
        }
        self.inner
            .begin_response(owner, request, authority, payload)?;
        // The production IMMEDIATE admission committed, but the writer has not started bytes.
        if self.race.timing == Timing::After {
            self.intervene(owner);
        }
        Ok(())
    }

    fn finish_response(
        &self,
        owner: (&str, u64),
        request: &ServerRequest,
        authority: &Value,
        payload: &Value,
        outcome: &str,
    ) -> Result<(), AppServerError> {
        self.inner
            .finish_response(owner, request, authority, payload, outcome)
    }
}
