use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    new_reply,
    prompt_intake::{self, NewPromptIntake},
    queue::{self, NewQueueJob},
};
use serde_json::json;
use std::path::{Path, PathBuf};

pub struct Fixture {
    pub _temp: tempfile::TempDir,
    pub db: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        admit(&db);
        Self { _temp: temp, db }
    }
    pub fn accept(&self) {
        let claimed = queue::try_begin_attempt(&self.db, "job", &[], 1)
            .unwrap()
            .unwrap();
        queue::mark_running_if_claimed(&self.db, &claimed, "first-turn")
            .unwrap()
            .unwrap();
    }
    pub fn verify(&self) {
        let record = new_reply::get(&self.db, "job").unwrap().unwrap();
        assert!(
            new_reply::checkpoint(
                &self.db,
                &record,
                new_reply::CheckpointUpdate {
                    scan: &json!({"proof":"fixture exact-turn scan"}),
                    verified: true,
                    error: "",
                    now: record.accepted_at.unwrap() + 1.0,
                }
            )
            .unwrap()
        );
    }
    pub fn ack(&self) {
        let record = new_reply::get(&self.db, "job").unwrap().unwrap();
        let key = new_reply::acknowledgement_key(&record).unwrap();
        assert_eq!(
            cdr_store::delivery_receipt::begin(
                &self.db,
                &key,
                &hash(&record.identity.acknowledgement)
            )
            .unwrap(),
            cdr_store::delivery_receipt::ReceiptState::New
        );
        cdr_store::delivery_receipt::confirm(&self.db, &key, "discord-ack").unwrap();
    }
}
pub fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}
fn admit(db: &Path) {
    let key = "message:30";
    ingress::admit(
        db,
        &NewIngress {
            ingress_id: key.into(),
            kind: IngressKind::Message,
            event_id: Some(30),
            application_id: Some(1),
            channel_id: 99,
            owner_user_id: 20,
            source_message_id: Some(30),
            payload: json!({"version":1,"plan":{"Execute":{"New":{"prompt":"요청"}}}}),
            target_thread_id: None,
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    ingress::begin_thread_start(db, key, 1, 2.0).unwrap();
    ingress::record_new_creation(db, key, 1, Some("C:/project"), 99, 2.0).unwrap();
    ingress::record_created_thread(db, key, 1, "new-thread", 3.0).unwrap();
    cdr_store::mapping::upsert_thread(db, "new-thread", "C:/project", "new", 99, 100, 3.0).unwrap();
    prompt_intake::admit_prompt_intake_with_ingress_and_reply(
        db,
        NewPromptIntake {
            job_id: "job",
            target_thread_id: "new-thread",
            channel_id: 100,
            owner_user_id: Some(20),
            discord_message_id: Some(30),
            raw_prompt: "요청",
            auto_queue_when_busy: true,
            require_current_mirror: true,
            created_at: 4.0,
        },
        key,
        1,
        Some(new_reply::NewReplySeed {
            state_db: Path::new("state.sqlite"),
            acknowledgement: "In progress\nmessage: 요청\n새 대화: <#100>",
        }),
    )
    .unwrap();
    let claim = prompt_intake::try_claim_prompt_intake(db, "job", 5.0, 600.0)
        .unwrap()
        .unwrap();
    prompt_intake::promote_prompt_intake_to_queue(
        db,
        &claim,
        NewQueueJob {
            job_id: "job",
            target_thread_id: "new-thread",
            channel_id: 100,
            owner_user_id: Some(20),
            discord_message_id: Some(30),
            app_server_generation: 1,
            prompt: "요청",
            queued: false,
            ack_sent: false,
            created_at: 4.0,
        },
        6.0,
    )
    .unwrap();
}
