//! IR1: real queue completion must persist release intent independently of delivery.
use super::*;

#[tokio::test]
async fn ir1_final_commit_keeps_exact_candidate_after_outbox_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let worker = goal_handoff_tests::make_worker(&temp).await;
    goal_handoff_tests::setup_running(&worker);
    let delivery = worker
        .queue
        .stage_turn_completion_on_generation("thread", "T1", "Final exact", 1)
        .await
        .unwrap()
        .unwrap();
    let owner = worker.server.instance_id().to_owned();
    worker.server.close().await.unwrap();
    assert!(
        cdr_store::queue::list(worker.queue.db_path())
            .unwrap()
            .is_empty()
    );
    assert_eq!(delivery.content, "Final exact");
    cdr_store::delivery::complete(worker.queue.db_path(), &delivery.delivery_id).unwrap();
    let db = rusqlite::Connection::open(worker.queue.db_path()).unwrap();
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_idle_release')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        exists,
        "IR1: Final committed without a durable release candidate"
    );
    let row: (String, String, String, String, String) = db
        .query_row(
            "SELECT owner_id,thread_id,turn_id,job_id,state FROM cdr_idle_release",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            owner,
            "thread".into(),
            "T1".into(),
            "job".into(),
            "Candidate".into()
        )
    );
}
