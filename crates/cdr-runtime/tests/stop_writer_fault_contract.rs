use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use cdr_app_server::writer_fault_fixture;
use cdr_store::queue::QueueJobState;
use serde_json::Value;
use support::{Fixture, INPUT, OTHER, Snapshot, TARGET, calls};

#[path = "support/stop_writer_fault_fixture.rs"]
mod support;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_partial_frame_accepts_stop_without_replay_on_healthy_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let original = Fixture::start(temp.path(), "partial-original", false).await;
    let barrier = writer_fault_fixture::arm_prefix(TARGET, 64).unwrap();
    let queue = Arc::clone(&original.queue);
    let task = tokio::spawn(async move { queue.submit(TARGET, 99, 20, Some(901), INPUT).await });
    let prefix = tokio::time::timeout(Duration::from_secs(5), barrier.wait_prefix())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(prefix.written, 64);
    assert!(prefix.written < prefix.frame_len);
    assert!(
        !task.is_finished(),
        "remainder has not been written or flushed"
    );
    let prepared = Snapshot::read(&original.db);
    prepared.assert_prepared();
    assert_eq!(
        prepared.attempts[0]["wire_id"].as_str(),
        Some(prefix.wire_id.to_string().as_str())
    );
    original.stop().await;
    let stopped = Snapshot::read(&original.db);
    stopped.assert_prepared();
    assert!(!stopped.holds.is_empty());
    assert_eq!(stopped.receipts.len(), 1);
    assert_eq!(
        calls(&original.log, "turn/start", TARGET),
        0,
        "prefix is not a complete JSON frame"
    );
    barrier.fail();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let accepted = result.unwrap();
    assert!(
        accepted.turn_id.is_none(),
        "durable acceptance is not a started turn"
    );
    assert_eq!(accepted.job_id, stopped.queue.job_id);
    stopped.assert_retained(&Snapshot::read(&original.db));
    let old_b = original
        .server
        .execute(
            cdr_app_server::requests::start_turn(OTHER, "must not reach broken generation"),
            Some(original.server.generation()),
        )
        .await;
    assert!(old_b.is_err());
    assert_eq!(calls(&original.log, "turn/start", OTHER), 0);
    original.server.close().await.unwrap();
    let replacement = Fixture::start(temp.path(), "partial-replacement", false).await;
    assert_ne!(
        original.server.instance_id(),
        replacement.server.instance_id()
    );
    replacement.prove_cold_no_replay(&stopped).await;
    replacement.server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abrupt_runtime_exit_keeps_prepared_and_stop_without_drop_or_replay() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::fs::File::create(temp.path().join("helper-output.log")).unwrap();
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "runtime_crash_helper",
            "--ignored",
            "--nocapture",
        ])
        .env("CDR_STOP_WRITER_HELPER_ROOT", temp.path())
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output))
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn().unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !temp.path().join("ready.json").is_file() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "helper exited before durable boundary"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let ready: Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("ready.json")).unwrap()).unwrap();
    assert_eq!(
        ready["stage"],
        "writer-and-stop-committed-before-first-byte"
    );
    let db = temp.path().join("mirror.sqlite");
    let before = Snapshot::read(&db);
    before.assert_prepared();
    assert_eq!(before.queue.state, QueueJobState::Starting);
    assert!(!before.holds.is_empty());
    assert_eq!(before.receipts.len(), 1);
    assert_eq!(before.attempts[0]["attempt_id"], ready["attempt"]);
    assert_eq!(before.attempts[0]["owner_id"], ready["owner"]);
    assert_eq!(before.attempts[0]["generation"], ready["generation"]);
    assert_eq!(
        calls(
            &temp.path().join("crash-original-rpc.jsonl"),
            "turn/start",
            TARGET
        ),
        0
    );
    std::fs::write(
        temp.path().join("exit-now"),
        b"parent independently observed committed evidence",
    )
    .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(73),
        "helper must exit without unwinding or graceful close"
    );
    assert_eq!(
        Snapshot::read(&db),
        before,
        "no Drop/finish may rewrite the prepared crash evidence"
    );
    let replacement = Fixture::start(temp.path(), "crash-replacement", false).await;
    assert_ne!(
        Some(replacement.server.instance_id()),
        ready["owner"].as_str()
    );
    replacement.prove_cold_no_replay(&before).await;
    replacement.server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "subprocess entry; exercised by abrupt_runtime_exit_keeps_prepared_and_stop_without_drop_or_replay"]
async fn runtime_crash_helper() {
    let root = PathBuf::from(
        std::env::var_os("CDR_STOP_WRITER_HELPER_ROOT")
            .expect("only the parent fixture may launch this helper"),
    );
    let fixture = Fixture::start(&root, "crash-original", true).await;
    let _ = fixture.queue.submit(TARGET, 99, 20, Some(901), INPUT).await;
    panic!("crash boundary was not reached");
}
