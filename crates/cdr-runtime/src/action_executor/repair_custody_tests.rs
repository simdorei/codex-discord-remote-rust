use super::*;
use crate::{action_executor::ActionContext, command_plan::CommandAction};

#[path = "recovery_custody_tests.rs"]
mod further;

fn context() -> ActionContext {
    ActionContext {
        channel_id: 42,
        user_id: 3,
        discord_message_id: Some(801),
        auto_queue_when_busy: false,
    }
}

fn admit(f: &MessageFixture, content: &str) -> crate::message_worker::AdmittedMessage {
    let admitted = f.admit(content);
    let record = cdr_store::ingress::get(f.executor.mirror_db(), "message:801")
        .unwrap()
        .unwrap();
    assert!(
        cdr_store::ingress::begin_execution(
            f.executor.mirror_db(),
            "message:801",
            "processing",
            record.target_thread_id.as_deref(),
            2.0
        )
        .unwrap()
    );
    admitted
}

fn remap(f: &MessageFixture) {
    let db = f.executor.mirror_db();
    cdr_store::mapping::upsert_thread(db, "thread-b", "project", "B", 100, 43, 3.0).unwrap();
    cdr_store::mapping::upsert_thread(db, "thread-a", "project", "A", 100, 42, 3.0).unwrap();
}

fn enqueue(f: &MessageFixture, job: &str, target: &str, event: i64) {
    cdr_store::queue::enqueue(
        f.executor.mirror_db(),
        cdr_store::queue::NewQueueJob {
            job_id: job,
            target_thread_id: target,
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(event),
            app_server_generation: 1,
            prompt: "must not replay",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
}

// The only host process these tests can launch is this inert temporary script.
fn fake_controller(temp: &tempfile::TempDir, f: &mut MessageFixture) {
    let root = temp.path().join("fake-controller");
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/Invoke-CdrDesktopRecovery.ps1"), r"
param($Mode,$RepoRoot,$CodexHome,$ThreadId,$ExpectedRecoveryIdentity)
$ErrorActionPreference='Stop'
Add-Content -LiteralPath (Join-Path $RepoRoot 'calls.txt') -Value ($Mode+':'+$ThreadId)
if($Mode -eq 'InspectTools') {
    @{ThreadId=$ThreadId;State='tools_recovery';RecoveryIdentity='fixture-only'} | ConvertTo-Json -Compress
} elseif($Mode -eq 'StartTools') {
    if($ExpectedRecoveryIdentity -ne 'fixture-only'){throw 'identity mismatch'}
    $op='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    @{Operation=$op;ReceiptPath=(Join-Path $RepoRoot ('maintenance_backups/desktop-recovery/'+$op+'/receipt.json'))} | ConvertTo-Json -Compress
} else {throw 'unexpected mode'}
").unwrap();
    f.executor.recovery_root = Some(root);
}

#[tokio::test]
async fn rr01_admitted_repair_remap_never_resets_a_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    let _admitted = admit(&f, "!repair");
    remap(&f);
    let result = f
        .executor
        .execute_with_ingress_context(
            CommandAction::Repair { reference: None },
            context(),
            "message:801",
        )
        .await;
    let sent = calls(&temp);
    f.server.close().await.unwrap();
    assert!(result.is_err(), "remapped admitted repair must be refused");
    assert_eq!(sent.len(), 1, "no read/reset/init/probe after remap");
}

#[cfg(windows)]
#[tokio::test]
async fn rr01_admitted_recover_remap_preserves_both_ledgers_and_never_restarts() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture(&temp, "ready").await;
    fake_controller(&temp, &mut f);
    let _admitted = admit(&f, "!recover");
    enqueue(&f, "original", "thread-b", 501);
    enqueue(&f, "replacement", "thread-a", 502);
    let before = cdr_store::queue::list(f.executor.mirror_db()).unwrap();
    remap(&f);
    let result = f
        .executor
        .execute_with_ingress_context(
            CommandAction::Recover { reference: None },
            context(),
            "message:801",
        )
        .await;
    let after = cdr_store::queue::list(f.executor.mirror_db()).unwrap();
    let host =
        std::fs::read_to_string(temp.path().join("fake-controller/calls.txt")).unwrap_or_default();
    f.server.close().await.unwrap();
    assert!(result.is_err(), "remapped admitted recover must be refused");
    assert_eq!(after, before);
    assert!(host.is_empty(), "no controller dispatch: {host}");
}

#[tokio::test]
async fn rr01_remap_after_inventory_wait_never_dispatches_reset() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "wait_inventory").await;
    let _admitted = admit(&f, "!repair");
    let command = f.executor.execute_with_ingress_context(
        CommandAction::Repair { reference: None },
        context(),
        "message:801",
    );
    let remapping = async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !calls(&temp)
                .iter()
                .any(|c| c["method"] == "mcpServerStatus/list")
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        remap(&f);
        std::fs::write(temp.path().join("rpc.release"), b"release").unwrap();
    };
    let (result, ()) = tokio::join!(command, remapping);
    let sent = calls(&temp);
    let unlocked = f
        .queue
        .target_lock("thread-b")
        .unwrap()
        .try_lock_owned()
        .is_ok();
    f.server.close().await.unwrap();
    assert!(result.is_err());
    assert!(!sent.iter().any(|c| c["method"] == "mcpServer/tool/call"));
    assert!(
        unlocked,
        "definite no-send refusal must release the target lock"
    );
}

#[tokio::test]
async fn rr01_recovery_admission_freezes_omitted_and_explicit_routes() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    for (index, text, target, route) in [
        (801, "!recover", "thread-b", "Mapped"),
        (802, "!repair", "thread-b", "Mapped"),
        (803, "!recover thread-a", "thread-a", "Explicit"),
        (804, "!repair thread-a", "thread-a", "Explicit"),
    ] {
        let _admitted = f.admit_id(text, index);
        let record = cdr_store::ingress::get(f.executor.mirror_db(), &format!("message:{index}"))
            .unwrap()
            .unwrap();
        assert_eq!(record.payload["lifecycle_binding"]["target"], target);
        assert_eq!(record.payload["lifecycle_binding"]["route"], route);
        assert_eq!(record.target_thread_id.as_deref(), Some(target));
    }
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn rr01_missing_legacy_binding_is_not_inferred_from_current_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    let _admitted = admit(&f, "!repair");
    rusqlite::Connection::open(f.executor.mirror_db()).unwrap().execute(
        "UPDATE discord_ingress_journal SET payload_json=json_remove(payload_json,'$.lifecycle_binding') WHERE ingress_id='message:801'", []).unwrap();
    let result = f
        .executor
        .execute_with_ingress_context(
            CommandAction::Repair { reference: None },
            context(),
            "message:801",
        )
        .await;
    let count = calls(&temp).len();
    f.server.close().await.unwrap();
    assert!(result.is_err());
    assert_eq!(count, 1);
}

#[tokio::test]
async fn rr01_duplicate_admitted_repair_has_one_effect_even_in_a_cold_executor() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp, "ready").await;
    let _admitted = admit(&f, "!repair");
    let action = CommandAction::Repair { reference: None };
    f.executor
        .execute_with_ingress_context(action.clone(), context(), "message:801")
        .await
        .unwrap();
    let cold = ActionExecutor::new(
        temp.path().join("state.sqlite"),
        temp.path().join("mirror.sqlite"),
        Arc::new(crate::bridge_state::BridgeState::new(
            temp.path().join("bridge.json"),
        )),
        Arc::clone(&f.queue),
    )
    .with_server(Arc::clone(&f.server));
    let duplicate = cold
        .execute_with_ingress_context(action, context(), "message:801")
        .await;
    let resets = calls(&temp)
        .iter()
        .filter(|c| c["params"]["tool"] == "js_reset")
        .count();
    f.server.close().await.unwrap();
    assert!(duplicate.is_err());
    assert_eq!(resets, 1);
}
