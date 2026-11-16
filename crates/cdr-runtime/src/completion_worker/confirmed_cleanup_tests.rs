//! Confirmed history cleanup must leave the async executor free for live work.
use super::*;
use cdr_store::{delivery, delivery_receipt};
use sha2::{Digest, Sha256};

#[tokio::test]
async fn confirmed_cleanup_waiting_for_sqlite_does_not_block_async_progress() {
    let temp = tempfile::tempdir().unwrap();
    let worker = goal_handoff_tests::make_worker(&temp).await;
    goal_handoff_tests::setup_running(&worker);
    let pending = worker
        .queue
        .stage_turn_completion_on_generation("thread", "T1", "Final exact", 1)
        .await
        .unwrap()
        .unwrap();
    let key = serde_json::to_string(&(42, "completion/v1", "job", 0)).unwrap();
    delivery_receipt::begin(
        worker.queue.db_path(),
        &key,
        &hex::encode(Sha256::digest(pending.content.as_bytes())),
    )
    .unwrap();
    delivery_receipt::confirm(worker.queue.db_path(), &key, "12345").unwrap();
    let db = rusqlite::Connection::open(worker.queue.db_path()).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (release, released) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        // Bound failure cleanup even if a synchronous SQLite wait blocks Tokio.
        let async_progress = released.recv_timeout(Duration::from_secs(1)).is_ok();
        db.execute_batch("ROLLBACK").unwrap();
        async_progress
    });
    let (result, ()) = tokio::join!(biased;
        worker.deliver_one(&pending),
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = release.send(());
        }
    );
    let progressed = writer.join().unwrap();
    worker.server.close().await.unwrap();
    result.unwrap();
    assert!(
        progressed,
        "confirmed cleanup blocked the async executor on SQLite"
    );
    assert!(
        delivery::list_pending(worker.queue.db_path())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        delivery_receipt::unknown_count(worker.queue.db_path()).unwrap(),
        0
    );
}
