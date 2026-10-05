use cdr_store::{
    async_resolution::{self, abandonment as a},
    queue,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::path::PathBuf;

pub const ID: &str = "cccccccccccccccccccccccccccccccc";
pub const JOB: &str = "b3d5a1a3-5c3e-4764-967b-0cef767efde9";
pub const TARGET: &str = async_resolution::REVIEWED_INCIDENT_THREAD;
pub const SIBLING: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub path: PathBuf,
    pub db: Connection,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private.sqlite");
        cdr_store::mapping::upsert_thread(&path, TARGET, "project", "title", 10, 20, 1.0).unwrap();
        for (job, event) in [(JOB, 70), (SIBLING, 71)] {
            queue::enqueue(
                &path,
                queue::NewQueueJob {
                    job_id: job,
                    target_thread_id: TARGET,
                    channel_id: 20,
                    owner_user_id: Some(30),
                    discord_message_id: Some(event),
                    app_server_generation: 1,
                    prompt: "private exact input\nKeep CASE",
                    queued: true,
                    ack_sent: true,
                    created_at: 1.0,
                },
            )
            .unwrap();
        }
        let db = cdr_store::schema::open_initialized(&path).unwrap();
        db.execute(
            "UPDATE codex_turn_queue SET attempt_count=356,baseline_turn_ids='[\"original\"]',
            last_error='original held evidence' WHERE job_id=?",
            [JOB],
        )
        .unwrap();
        db.execute(
            "INSERT INTO codex_app_server_runtime VALUES(1,'app-fixture')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO codex_mutation_runtime VALUES(1,'wire-fixture')",
            [],
        )
        .unwrap();
        let payload = json!({
            "version":1, "content":format!("!discard-request {JOB}"),
            "plan":{"Execute":{"DiscardRequest":{"job_id":JOB}}},
            "author_is_bot":false,
        });
        db.execute("INSERT INTO discord_ingress_journal
            (ingress_id,kind,event_id,channel_id,owner_user_id,source_message_id,payload_json,
             runtime_id,state,phase,target_thread_id,created_at,updated_at)
             VALUES('message:80','message',80,20,30,80,?,'app-fixture','executing','processing',?,2,2)",
            params![payload.to_string(), TARGET],
        ).unwrap();
        Self { temp, path, db }
    }

    pub fn propose(&self) -> a::Proposal {
        a::propose(
            &self.path,
            &a::ProposalInput {
                proposal_id: ID,
                job_id: JOB,
                ingress_id: "message:80",
                application_id: 50,
                now: 10.0,
                expires_at: 100.0,
            },
        )
        .unwrap()
    }

    pub fn delivered(&self) -> a::Proposal {
        let p = self.propose();
        a::bind_delivery(&self.path, ID, 60, &p.review_sha256, 11.0).unwrap();
        p
    }

    pub fn click(&self, decision: a::Decision) {
        let name = match decision {
            a::Decision::AbandonOnly => "AbandonOnly",
            a::Decision::KeepHeld => "KeepHeld",
        };
        let payload = json!({"version":1,"work":{"Component":{"RecoveryAbandonDecision":{
            "proposal_id":ID,"revision":1,"decision":name
        }}}});
        self.db
            .execute(
                "INSERT INTO discord_ingress_journal
            (ingress_id,kind,event_id,application_id,channel_id,owner_user_id,source_message_id,
             payload_json,runtime_id,state,phase,target_thread_id,created_at,updated_at)
             VALUES('interaction:90','interaction',90,50,20,30,60,?,'app-fixture',
                'executing','processing',?,12,12)",
                params![payload.to_string(), TARGET],
            )
            .unwrap();
    }

    pub fn apply(&self, decision: a::Decision) -> cdr_store::Result<a::DecisionReceipt> {
        a::record_decision(
            &self.path,
            &a::DecisionInput {
                proposal_id: ID,
                revision: 1,
                ingress_id: "interaction:90",
                decision,
                now: 13.0,
            },
        )
    }

    pub fn count(&self, table: &str) -> i64 {
        self.db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    pub fn unchanged(&self) {
        assert_eq!(self.count("codex_turn_queue"), 2);
        assert_eq!(self.count("codex_request_cancellations"), 0);
        assert_eq!(self.count("cdr_recovery_abandonment_decisions"), 0);
        assert!(async_resolution::admission_held(&self.path, TARGET).unwrap());
    }
}
