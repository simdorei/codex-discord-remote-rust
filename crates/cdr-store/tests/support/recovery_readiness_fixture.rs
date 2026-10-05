use cdr_store::{
    async_question as aq,
    async_resolution::{self, abandonment as a},
    queue,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::PathBuf;

use super::orphan;

pub const ID: &str = "dddddddddddddddddddddddddddddddd";
pub const JOB: &str = "b3d5a1a3-5c3e-4764-967b-0cef767efde9";
pub const SIBLING: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
pub const TARGET: &str = "thread-b";
pub const RUNTIME: &str = "app-fixture";

pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub path: PathBuf,
    pub db: Connection,
    pub question: String,
    runtime: String,
}

impl Fixture {
    pub fn new(choice: a::Decision) -> Self {
        Self::with_runtime(choice, RUNTIME)
    }

    pub fn with_runtime(choice: a::Decision, runtime: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private.sqlite");
        let question = orphan::dispatching(&path, runtime);
        aq::record_error(&path, &question, "preserved original timeout").unwrap();
        for (job, event) in [(JOB, 70), (SIBLING, 71)] {
            orphan::pending(&path, job, TARGET, 1);
            Connection::open(&path)
                .unwrap()
                .execute(
                    "UPDATE codex_turn_queue SET discord_message_id=?,attempt_count=356,
                 baseline_turn_ids='[\"original\"]',prompt='private preserved input',
                 last_error='preserved pending evidence' WHERE job_id=?",
                    params![event, job],
                )
                .unwrap();
        }
        let db = Connection::open(&path).unwrap();
        db.execute(
            "INSERT INTO codex_app_server_runtime VALUES(1,?)",
            [runtime],
        )
        .unwrap();
        db.execute(
            "INSERT INTO codex_mutation_runtime VALUES(1,'wire-fixture')",
            [],
        )
        .unwrap();
        db.execute("UPDATE cdr_async_execution_obligations SET policy='publishing_recovery' WHERE question_id=?",
            [&question]).unwrap();
        db.execute("INSERT INTO cdr_async_recovery_policies VALUES(?,1,'publishing_recovery',?,'original','origin',?)",
            params![TARGET, async_resolution::REVIEWED_PROPOSAL_SHA256, JOB]).unwrap();
        let source = json!({"version":1,"author_is_bot":false,
            "content":format!("!discard-request {JOB}"),
            "plan":{"Execute":{"DiscardRequest":{"job_id":JOB}}}});
        db.execute(
            "INSERT INTO discord_ingress_journal
            (ingress_id,kind,event_id,channel_id,owner_user_id,source_message_id,payload_json,
             runtime_id,state,phase,target_thread_id,created_at,updated_at)
            VALUES('message:80','message',80,20,30,80,?,?,'executing','processing',?,2,2)",
            params![source.to_string(), runtime, TARGET],
        )
        .unwrap();
        let proposal = a::propose(
            &path,
            &a::ProposalInput {
                proposal_id: ID,
                job_id: JOB,
                ingress_id: "message:80",
                application_id: 50,
                now: 10.0,
                expires_at: 100.0,
            },
        )
        .unwrap();
        a::bind_delivery(&path, ID, 60, &proposal.review_sha256, 11.0).unwrap();
        let click = json!({"version":1,"work":{"Component":{"RecoveryAbandonDecision":{
            "proposal_id":ID,"revision":1,"decision":choice
        }}}});
        db.execute("INSERT INTO discord_ingress_journal
            (ingress_id,kind,event_id,application_id,channel_id,owner_user_id,source_message_id,
             payload_json,runtime_id,state,phase,target_thread_id,created_at,updated_at)
            VALUES('interaction:90','interaction',90,50,20,30,60,?,?,'executing','processing',?,12,12)",
            params![click.to_string(), runtime, TARGET]).unwrap();
        a::record_decision(
            &path,
            &a::DecisionInput {
                proposal_id: ID,
                revision: 1,
                ingress_id: "interaction:90",
                decision: choice,
                now: 13.0,
            },
        )
        .unwrap();
        Self {
            temp,
            path,
            db,
            question,
            runtime: runtime.into(),
        }
    }

    pub fn settle(&self, turn: &str) -> Value {
        queue::complete(&self.path, "origin").unwrap();
        let snapshot = async_resolution::capture_terminal_history_snapshot(&self.path, TARGET)
            .unwrap()
            .expect("real original obligation was not captured");
        let observed = observation(turn);
        assert_eq!(
            async_resolution::settle_terminal_history(
                &self.path,
                &snapshot,
                &observed,
                &self.runtime,
                1,
            )
            .unwrap(),
            1
        );
        observed
    }

    pub fn successor(&self) {
        async_resolution::record_terminal_notification(
            &self.path,
            TARGET,
            "original",
            1,
            &self.runtime,
            &json!({"threadId":TARGET,"turn":{"id":"original","status":"completed"}}).to_string(),
        )
        .unwrap();
        assert!(queue::mark_goal_waiting(&self.path, "origin", "original", 1).unwrap());
        let waiting = queue::list(&self.path)
            .unwrap()
            .into_iter()
            .find(|job| job.job_id == "origin")
            .unwrap();
        assert!(
            queue::attach_goal_turn_observed_if_owned(&self.path, &waiting, "successor", 1)
                .unwrap()
        );
    }

    pub fn state(&self) -> Value {
        let question = aq::get(&self.path, &self.question).unwrap();
        let obligations: Vec<(i64, String, String, Option<String>)> = self
            .db
            .prepare(
                "SELECT revision,execution_state,admission_state,terminal_proof_json
             FROM cdr_async_execution_obligations ORDER BY question_id",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        json!({"jobs":queue::list(&self.path).unwrap(),"obligations":obligations,
            "question_state":question.state,"question_error":question.error,
            "decision":a::decision_status(&self.path, ID, 1).unwrap()})
    }

    pub fn held(&self) {
        assert!(async_resolution::admission_held(&self.path, TARGET).unwrap());
        assert!(async_resolution::guard_mutation(&self.path, TARGET).is_err());
        let before = queue::list(&self.path).unwrap();
        assert!(queue::try_begin_attempt(&self.path, SIBLING, &[], 1).is_err());
        assert_eq!(queue::list(&self.path).unwrap(), before);
    }
}

pub fn observation(turn: &str) -> Value {
    json!({"threadId":TARGET,"truncated":false,
        "thread_observation":{"thread":{"id":TARGET,"status":{"type":"idle"}}},
        "goal_observation":{"goal":null},
        "turns":[{"id":turn,"status":"completed","items":[]}]})
}
