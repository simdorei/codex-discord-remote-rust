use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cdr_app_server::{AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId};
use cdr_runtime::dead_generation_recovery::RuntimeDeadGenerationFence;
use cdr_store::{
    ingress::stop::{StopScope, accept_nonrunning},
    queue::StoredQueueJob,
};
use rusqlite::{Connection, OptionalExtension};
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct Journal {
    pub state: String,
    digest: String,
    attempt: String,
    wire: String,
    generation: i64,
    owner: String,
}

pub fn journal(path: &Path) -> Option<Journal> {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT state,request_sha256,attempt_id,wire_id,generation,owner_id
         FROM codex_mutation_attempts WHERE target_thread_id='thread-b' AND method='turn/start'",
            [],
            |row| {
                Ok(Journal {
                    state: row.get(0)?,
                    digest: row.get(1)?,
                    attempt: row.get(2)?,
                    wire: row.get(3)?,
                    generation: row.get(4)?,
                    owner: row.get(5)?,
                })
            },
        )
        .optional()
        .unwrap()
}

#[derive(Clone)]
pub struct Captured {
    pub job: StoredQueueJob,
    pub claim: Value,
    pub params: Value,
    pub prepared: Option<Journal>,
}

pub struct OrderedFence {
    inner: RuntimeDeadGenerationFence,
    db: PathBuf,
    race: Race,
    fired: AtomicBool,
    pub captured: Mutex<Option<Captured>>,
}

impl OrderedFence {
    pub fn new(inner: RuntimeDeadGenerationFence, db: PathBuf, race: Race) -> Self {
        Self {
            inner,
            db,
            race,
            fired: AtomicBool::new(false),
            captured: Mutex::new(None),
        }
    }

    fn stop(&self, original: &str) {
        let accepted = accept_nonrunning(
            &self.db, StopScope { target:"thread-b", channel:42, owner:3 },
            &json!({"target":"thread-b","route":"Explicit","command":{"Stop":{"reference":"thread-b"}}}),
            None, || Ok(()),
        ).unwrap().expect("the original Starting request must accept stop");
        assert_eq!(accepted.jobs, [original]);
    }

    fn recover(&self, original: &str) {
        let result = cdr_store::queue::cancel_for_recovery(&self.db, "thread-b", 42, 3, 2.0);
        if self.race.action == Action::RecoveryRollback {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("injected start recovery rollback"),
                "{error}"
            );
        } else {
            assert_eq!(result.unwrap().jobs, [original]);
        }
    }

    fn intervene(&self, original: &str) {
        match self.race.action {
            Action::Stop => self.stop(original),
            Action::Recovery => self.recover(original),
            Action::StopThenRecovery => {
                self.stop(original);
                let prior = cdr_store::execution_hold::reason(&self.db, original).unwrap();
                self.recover(original);
                assert_eq!(
                    cdr_store::execution_hold::reason(&self.db, original).unwrap(),
                    prior
                );
            }
            Action::RecoveryRollback => {
                Connection::open(&self.db)
                    .unwrap()
                    .execute_batch(
                        "CREATE TRIGGER reject_start_recovery BEFORE DELETE ON codex_turn_queue
                     WHEN OLD.target_thread_id='thread-b'
                     BEGIN SELECT RAISE(ABORT,'injected start recovery rollback'); END;",
                    )
                    .unwrap();
                self.recover(original);
            }
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

    fn begin_queue_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        method: &str,
        params: &Value,
        claim: &Value,
    ) -> Result<bool, AppServerError> {
        if params["threadId"] != "thread-b" || self.fired.swap(true, Ordering::AcqRel) {
            return self
                .inner
                .begin_queue_mutation(owner, request, method, params, claim);
        }
        assert_eq!(method, "turn/start");
        let jobs = cdr_store::queue::list_filtered(&self.db, Some("thread-b"), None).unwrap();
        assert_eq!(jobs.len(), 1);
        let job = jobs[0].clone();
        assert_eq!(job.state, cdr_store::queue::QueueJobState::Starting);
        *self.captured.lock().unwrap() = Some(Captured {
            job: job.clone(),
            claim: claim.clone(),
            params: params.clone(),
            prepared: None,
        });
        if self.race.timing == Timing::Before {
            self.intervene(&job.job_id);
        }
        let accepted = self
            .inner
            .begin_queue_mutation(owner, request, method, params, claim)?;
        assert!(accepted);
        let prepared = journal(&self.db).expect("production writer admission must be durable");
        assert_eq!(prepared.state, "prepared");
        self.captured.lock().unwrap().as_mut().unwrap().prepared = Some(prepared);
        // Real production admission committed; the actual writer has not written bytes.
        if self.race.timing == Timing::After {
            self.intervene(&job.job_id);
        }
        Ok(accepted)
    }
}
