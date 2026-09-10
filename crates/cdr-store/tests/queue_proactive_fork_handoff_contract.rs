use cdr_store::mapping::{thread_channels, upsert_thread};
use cdr_store::queue::{
    AppServerForkHandoffError, NewAppServerForkHandoff, NewQueueJob, QueueJobState,
    begin_app_server_fork_handoff, complete_app_server_fork_handoff, enqueue,
    is_app_server_managed_target, list, record_preflight_failure,
    unresolved_app_server_fork_handoff_for_source,
};

#[test]
fn proactive_fork_without_ambiguous_job_moves_only_pending_work() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("proactive.sqlite");
    upsert_thread(&path, "ordinary", "project", "Ordinary", 500, 501, 1.0)
        .expect("map ordinary mirror");
    enqueue(
        &path,
        NewQueueJob {
            job_id: "pending",
            target_thread_id: "ordinary",
            channel_id: 501,
            owner_user_id: Some(55),
            discord_message_id: Some(601),
            app_server_generation: 3,
            prompt: "preserve me",
            queued: true,
            ack_sent: true,
            created_at: 2.0,
        },
    )
    .expect("enqueue pending work");
    record_preflight_failure(&path, "pending", 3, "old writer conflict")
        .expect("record stale source backoff");

    let begun = begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "proactive-a",
            ambiguous_job_id: None,
            source_thread_id: "ordinary",
            expected_generation: 3,
            quarantine_reason: "proactive app-server ownership fork",
        },
    )
    .expect("persist proactive intent");
    assert!(begun.created);
    assert_eq!(begun.handoff.ambiguous_job_id, None);
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "ordinary")
            .unwrap()
            .is_some()
    );

    let completed = complete_app_server_fork_handoff(&path, "proactive-a", "managed", 8)
        .expect("complete proactive fork");
    assert!(completed.applied);
    assert_eq!(completed.quarantined_job, None);
    assert_eq!(completed.retargeted_jobs.len(), 1);
    let jobs = list(&path).expect("list moved work");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Pending);
    assert_eq!(jobs[0].target_thread_id, "managed");
    assert_eq!(jobs[0].app_server_generation, 8);
    assert_eq!(jobs[0].prompt, "preserve me");
    assert_eq!(jobs[0].discord_message_id, Some(601));
    assert_eq!(jobs[0].attempt_count, 1);
    assert!(jobs[0].last_error.is_empty());
    assert_eq!(thread_channels(&path, "ordinary").unwrap(), None);
    assert_eq!(thread_channels(&path, "managed").unwrap(), Some((500, 501)));
    assert!(is_app_server_managed_target(&path, "managed").unwrap());
    let second_intent = begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "must-not-refork",
            ambiguous_job_id: None,
            source_thread_id: "ordinary",
            expected_generation: 8,
            quarantine_reason: "must remain fenced",
        },
    );
    assert!(matches!(
        second_intent,
        Err(AppServerForkHandoffError::ConflictingIntent { .. })
    ));
}

#[test]
fn proactive_fork_with_no_jobs_still_transfers_mapping_ownership() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("empty.sqlite");
    upsert_thread(&path, "ordinary", "project", "Ordinary", 510, 511, 1.0)
        .expect("map ordinary mirror");
    begin_app_server_fork_handoff(
        &path,
        NewAppServerForkHandoff {
            handoff_id: "proactive-empty",
            ambiguous_job_id: None,
            source_thread_id: "ordinary",
            expected_generation: 4,
            quarantine_reason: "proactive app-server ownership fork",
        },
    )
    .expect("persist empty proactive intent");
    let completed = complete_app_server_fork_handoff(&path, "proactive-empty", "managed", 9)
        .expect("complete empty proactive fork");
    assert_eq!(completed.quarantined_job, None);
    assert!(completed.retargeted_jobs.is_empty());
    assert!(is_app_server_managed_target(&path, "managed").unwrap());
}
