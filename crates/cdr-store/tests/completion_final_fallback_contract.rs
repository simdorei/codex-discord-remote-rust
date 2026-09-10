use cdr_store::observed_final_answer;
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, mark_running};

fn running(path: &std::path::Path, generation: i64) {
    enqueue(
        path,
        NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 1,
            owner_user_id: Some(2),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(path, "job", &[], generation).unwrap();
    mark_running(path, "job", "turn", generation).unwrap();
}

#[test]
fn exact_final_is_durable_and_rejects_wrong_thread_turn_or_generation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    running(&path, 7);

    assert!(!observed_final_answer::record(&path, "other", "turn", 7, "wrong").unwrap());
    assert!(!observed_final_answer::record(&path, "thread", "other", 7, "wrong").unwrap());
    assert!(!observed_final_answer::record(&path, "thread", "turn", 6, "wrong").unwrap());
    assert!(observed_final_answer::record(&path, "thread", "turn", 7, "exact").unwrap());
    assert!(!observed_final_answer::record(&path, "thread", "turn", 7, "changed").unwrap());
    assert_eq!(
        observed_final_answer::get(&path, "thread", "turn", 7).unwrap(),
        Some("exact".into())
    );
    assert_eq!(
        observed_final_answer::get(&path, "thread", "turn", 6).unwrap(),
        None
    );

    assert_eq!(
        observed_final_answer::get(&path, "thread", "turn", 7).unwrap(),
        Some("exact".into()),
        "a reopened connection must retain the exact final"
    );
}
