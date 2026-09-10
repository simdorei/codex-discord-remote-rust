use cdr_store::StoreError;
use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    NewAppServerForkHandoff, NewQueueJob, begin_app_server_fork_handoff,
    complete_app_server_fork_handoff, enqueue, list, retract,
};

#[test]
fn completed_handoff_winning_before_retract_returns_the_destination_and_keeps_the_job() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("completed-before-retract.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(
        &path,
        NewQueueJob {
            job_id: "pending",
            target_thread_id: "source",
            channel_id: 101,
            owner_user_id: Some(7),
            discord_message_id: Some(10),
            app_server_generation: 4,
            prompt: "keep",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "handoff",
            ambiguous_job_id: None,
            source_thread_id: "source",
            expected_generation: 4,
            quarantine_reason: "ownership fork",
        },
    )
    .unwrap();
    complete_app_server_fork_handoff(&path, "handoff", "destination", 9).unwrap();

    let result = retract(&path, "source", Some(101), Some(7));
    assert!(matches!(
        result,
        Err(StoreError::ForkHandoffTargetMoved {
            source_thread_id,
            target_thread_id,
        }) if source_thread_id == "source" && target_thread_id == "destination"
    ));
    let jobs = list(&path).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "pending");
    assert_eq!(jobs[0].target_thread_id, "destination");
}
