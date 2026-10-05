#![cfg(windows)]

use cdr_runtime::writer_recovery::{self, Target};
use cdr_store::{
    queue::{self, NewQueueJob},
    schema::open_initialized,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn fixture(mode: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join("scripts")).unwrap();
    std::fs::write(root.join("fixture-mode"), mode).unwrap();
    std::fs::write(root.join("scripts/Invoke-CdrDesktopRecovery.ps1"), r"
param([string]$Mode,[string]$RepoRoot,[string]$CodexHome,[string]$ThreadId,[string]$ExpectedRecoveryIdentity)
$ErrorActionPreference='Stop'
if($Mode -ceq 'InspectTools') {
    @{ThreadId=$ThreadId;State='tools_recovery';RecoveryIdentity='desktop;bot;writer'} | ConvertTo-Json -Compress
    exit 0
}
if($Mode -cne 'StartTools' -or $ExpectedRecoveryIdentity -cne 'desktop;bot;writer') { throw 'Unexpected fixture dispatch' }
[IO.File]::AppendAllText((Join-Path $RepoRoot 'fixture-dispatches'),('start'+[Environment]::NewLine))
$operation='abcdef0123456789abcdef0123456789'
$bundle=Join-Path $RepoRoot ('maintenance_backups/desktop-recovery/'+$operation)
[void][IO.Directory]::CreateDirectory($bundle)
$receipt=Join-Path $bundle 'receipt.json'
[IO.File]::WriteAllText($receipt,(@{Phase='dispatched';Operation=$operation;ThreadId=$ThreadId;Evidence='accepted but unconfirmed'} | ConvertTo-Json))
$case=[IO.File]::ReadAllText((Join-Path $RepoRoot 'fixture-mode'))
if($case -ceq 'fail') { [Console]::Error.WriteLine('injected controller failure after handoff');exit 1 }
if($case -ceq 'malformed') { Write-Output '{invalid reply';exit 0 }
@{Operation=$operation;ReceiptPath=$receipt;WorkerPid=4242} | ConvertTo-Json -Compress
").unwrap();
    let db = root.join("fixture.sqlite");
    (temp, db)
}

fn seed(database: &Path, owner: i64) {
    queue::enqueue(
        database,
        NewQueueJob {
            job_id: "recover-request",
            target_thread_id: "recover-a",
            channel_id: 42,
            owner_user_id: Some(owner),
            discord_message_id: Some(71),
            app_server_generation: 1,
            prompt: "preserve exact original input",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
}

async fn run(root: &Path, database: &Path, channel: i64) -> Result<Value, String> {
    writer_recovery::run_tools(
        Target {
            root,
            codex_home: root,
            database,
            thread: "recover-a",
            channel,
            user: 3,
        },
        false,
    )
    .await
}

fn assert_cancelled_without_replay(database: &Path) {
    assert!(queue::list(database).unwrap().is_empty());
    let db = open_initialized(database).unwrap();
    let cancellations: i64 = db
        .query_row(
            "SELECT count(*) FROM codex_request_cancellations WHERE job_id='recover-request'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cancellations, 1);
    let evidence: String = db
        .query_row(
            "SELECT evidence_json FROM cdr_execution_holds WHERE job_id='recover-request'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(evidence.contains("preserve exact original input"));
    assert!(
        queue::try_begin_attempt(database, "recover-request", &[], 1)
            .unwrap()
            .is_none()
    );
    assert!(
        queue::enqueue(
            database,
            NewQueueJob {
                job_id: "new-id-for-old-request",
                target_thread_id: "recover-a",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(71),
                app_server_generation: 2,
                prompt: "must not replay",
                queued: true,
                ack_sent: true,
                created_at: 2.0,
            }
        )
        .is_err()
    );
}

#[tokio::test]
async fn cancellation_survives_controller_failure_and_lost_response() {
    for mode in ["fail", "malformed"] {
        let (temp, db) = fixture(mode);
        seed(&db, 3);
        assert!(run(temp.path(), &db, 42).await.is_err());
        assert_cancelled_without_replay(&db);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("fixture-dispatches"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        let receipt = temp.path().join(
            "maintenance_backups/desktop-recovery/abcdef0123456789abcdef0123456789/receipt.json",
        );
        let outcome: Value = serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
        assert_eq!(outcome["Phase"], "dispatched");
        assert_eq!(outcome["Evidence"], "accepted but unconfirmed");
    }
}

#[tokio::test]
async fn foreign_sender_or_channel_prevents_host_dispatch_and_preserves_requests() {
    for (channel, owner) in [(42, 4), (43, 3)] {
        let (temp, db) = fixture("ok");
        seed(&db, owner);
        let before = serde_json::to_value(queue::list(&db).unwrap()).unwrap();
        assert!(run(temp.path(), &db, channel).await.is_err());
        assert_eq!(
            serde_json::to_value(queue::list(&db).unwrap()).unwrap(),
            before
        );
        assert!(!temp.path().join("fixture-dispatches").exists());
        let count: i64 = open_initialized(&db)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM codex_request_cancellations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn cancellation_insert_failure_rolls_back_and_prevents_host_dispatch() {
    let (temp, db) = fixture("ok");
    seed(&db, 3);
    let before = serde_json::to_value(queue::list(&db).unwrap()).unwrap();
    open_initialized(&db).unwrap().execute_batch(
        "CREATE TRIGGER reject_recovery_cancellation BEFORE INSERT ON codex_request_cancellations
         BEGIN SELECT RAISE(ABORT,'injected cancellation write failure'); END;"
    ).unwrap();
    assert!(run(temp.path(), &db, 42).await.is_err());
    assert_eq!(
        serde_json::to_value(queue::list(&db).unwrap()).unwrap(),
        before
    );
    assert!(!temp.path().join("fixture-dispatches").exists());
    let evidence: i64 = open_initialized(&db)
        .unwrap()
        .query_row("SELECT count(*) FROM cdr_execution_holds", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(evidence, 0);
}

#[tokio::test]
async fn successful_handoff_stays_unconfirmed_and_never_replays_the_original_request() {
    let (temp, db) = fixture("ok");
    seed(&db, 3);
    let result = run(temp.path(), &db, 42).await.unwrap();
    assert_eq!(result["phase"], "dispatched_tools");
    assert!(result.get("ToolProbeVerified").is_none());
    assert_cancelled_without_replay(&db);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("fixture-dispatches"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn full_recovery_result_warns_that_other_work_may_be_interrupted() {
    let message = writer_recovery::message(&json!({"phase":"dispatched_tools","cancelled":[]}));
    assert!(
        message.contains("다른") && message.contains("중단"),
        "{message}"
    );
}

fn admit_preparer(database: &Path, owner: i64) {
    cdr_store::ingress::admit(
        database,
        &cdr_store::ingress::NewIngress {
            ingress_id: "message:75".into(),
            kind: cdr_store::ingress::IngressKind::Message,
            event_id: Some(75),
            application_id: None,
            channel_id: 42,
            owner_user_id: owner,
            source_message_id: Some(75),
            target_thread_id: Some("recover-a".into()),
            payload: json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"unowned original"}}}}),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
    assert!(
        cdr_store::ingress::begin_execution(database, "message:75", "preparing", None, 2.0)
            .unwrap()
    );
}

#[tokio::test]
async fn all_five_request_states_cancel_together_without_touching_other_target() {
    let (temp, db) = fixture("ok");
    for (id, event, target, channel, owner) in [
        ("pending", 71, "recover-a", 42, 3),
        ("starting", 72, "recover-a", 42, 3),
        ("running", 73, "recover-a", 42, 3),
        ("unrelated", 76, "recover-b", 43, 4),
    ] {
        queue::enqueue(
            &db,
            NewQueueJob {
                job_id: id,
                target_thread_id: target,
                channel_id: channel,
                owner_user_id: Some(owner),
                discord_message_id: Some(event),
                app_server_generation: 1,
                prompt: "mixed original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
    }
    queue::try_begin_attempt(&db, "starting", &[], 1)
        .unwrap()
        .unwrap();
    queue::try_begin_attempt(&db, "running", &[], 1)
        .unwrap()
        .unwrap();
    queue::mark_running(&db, "running", "original-running-turn", 1).unwrap();
    cdr_store::prompt_intake::admit_prompt_intake(
        &db,
        cdr_store::prompt_intake::NewPromptIntake {
            job_id: "intake",
            target_thread_id: "recover-a",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(74),
            raw_prompt: "intake original",
            auto_queue_when_busy: true,
            require_current_mirror: false,
            created_at: 1.0,
        },
    )
    .unwrap();
    admit_preparer(&db, 3);
    let unrelated = queue::list(&db)
        .unwrap()
        .into_iter()
        .find(|job| job.job_id == "unrelated")
        .unwrap();
    let report = run(temp.path(), &db, 42).await.unwrap();
    assert_eq!(report["cancelled"].as_array().unwrap().len(), 5);
    assert_eq!(report["started_or_uncertain"], 3);
    assert_eq!(queue::list(&db).unwrap(), vec![unrelated]);
    assert!(
        cdr_store::prompt_intake::list_prompt_intakes(&db)
            .unwrap()
            .is_empty()
    );
    let connection = open_initialized(&db).unwrap();
    for table in ["codex_request_cancellations", "cdr_execution_holds"] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 5, "{table}");
    }
    let evidence: String = connection
        .query_row(
            "SELECT evidence_json FROM cdr_execution_holds WHERE job_id='running'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        evidence.contains("original-running-turn") && evidence.contains("mixed original input")
    );
    assert!(
        queue::try_begin_attempt(&db, "pending", &[], 1)
            .unwrap()
            .is_none()
    );
    assert!(queue::mark_running(&db, "starting", "late-turn", 1).is_err());
    assert!(
        !cdr_store::ingress::begin_execution(&db, "message:75", "late promotion", None, 3.0)
            .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("fixture-dispatches"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[tokio::test]
async fn foreign_unowned_preparer_rolls_back_already_staged_queue_cancellation() {
    let (temp, db) = fixture("ok");
    seed(&db, 3);
    admit_preparer(&db, 4);
    let before_queue = queue::list(&db).unwrap();
    let before_ingress = cdr_store::ingress::get(&db, "message:75").unwrap();
    assert!(run(temp.path(), &db, 42).await.is_err());
    assert_eq!(queue::list(&db).unwrap(), before_queue);
    assert_eq!(
        cdr_store::ingress::get(&db, "message:75").unwrap(),
        before_ingress
    );
    assert!(!temp.path().join("fixture-dispatches").exists());
    let connection = open_initialized(&db).unwrap();
    for table in ["codex_request_cancellations", "cdr_execution_holds"] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}
