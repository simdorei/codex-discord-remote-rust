use cdr_store::StoreError;
use cdr_store::mapping::{thread_channels, upsert_thread};
use cdr_store::queue::{
    ExpectedMirrorMapping, NewAppServerForkHandoff, NewQueueJob, begin_app_server_fork_handoff,
    complete_app_server_fork_handoff, enqueue, enqueue_if_mirror_matches, list,
    record_app_server_fork_failure,
};

#[test]
fn enqueue_before_handoff_is_moved_by_the_same_handoff_transaction() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("enqueue-first.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();

    enqueue_if_mirror_matches(
        &path,
        job("pending", "source", 101, 1),
        ExpectedMirrorMapping {
            discord_channel_id: 101,
            target_thread_id: "source",
        },
    )
    .expect("current exact mirror mapping permits enqueue");
    begin_app_server_fork_handoff(&path, request("enqueue-first")).expect("fence ownership fork");
    complete_app_server_fork_handoff(&path, "enqueue-first", "fork", 9)
        .expect("handoff retargets already committed pending job");

    let jobs = list(&path).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, "fork");
    assert_eq!(jobs[0].app_server_generation, 9);
}

#[test]
fn handoff_before_enqueue_rejects_the_stale_mapping_and_duplicate_read() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("handoff-first.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    cdr_store::queue::enqueue(&path, job("dedupe", "source", 101, 42)).unwrap();
    begin_app_server_fork_handoff(&path, request("handoff-first")).unwrap();
    complete_app_server_fork_handoff(&path, "handoff-first", "fork", 9).unwrap();
    assert_eq!(thread_channels(&path, "fork").unwrap(), Some((100, 101)));

    let result = enqueue_if_mirror_matches(
        &path,
        job("stale-new-id", "source", 101, 42),
        ExpectedMirrorMapping {
            discord_channel_id: 101,
            target_thread_id: "source",
        },
    );
    assert!(matches!(
        result,
        Err(StoreError::MirrorMappingChanged {
            discord_channel_id: 101,
            expected_target_thread_id,
            actual_target_thread_id: Some(actual),
        }) if expected_target_thread_id == "source" && actual == "fork"
    ));
    assert!(
        list(&path)
            .unwrap()
            .iter()
            .all(|job| job.job_id != "stale-new-id")
    );
}

#[test]
fn duplicate_or_ambiguous_discord_mapping_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("duplicate.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    upsert_thread(&path, "duplicate", "project", "Duplicate", 200, 101, 2.0).unwrap();

    let result = enqueue_if_mirror_matches(
        &path,
        job("must-not-enqueue", "source", 101, 8),
        ExpectedMirrorMapping {
            discord_channel_id: 101,
            target_thread_id: "source",
        },
    );
    assert!(matches!(
        result,
        Err(StoreError::MirrorMappingChanged {
            discord_channel_id: 101,
            expected_target_thread_id,
            actual_target_thread_id: None,
        }) if expected_target_thread_id == "source"
    ));
    assert!(list(&path).unwrap().is_empty());
}

#[test]
fn plain_enqueue_rejects_an_unresolved_handoff_even_before_an_error_is_recorded() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plain-unresolved.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    begin_app_server_fork_handoff(&path, request("plain-unresolved")).unwrap();

    let result = enqueue(&path, job("must-not-enqueue", "source", 101, 50));
    assert!(matches!(
        result,
        Err(StoreError::ForkHandoffUnresolved {
            target_thread_id,
            last_error: None,
        }) if target_thread_id == "source"
    ));
    assert!(list(&path).unwrap().is_empty());
}

#[test]
fn prepared_guarded_enqueue_rechecks_after_an_ambiguous_handoff_wins() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("guarded-unresolved.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    let prepared = job("must-not-enqueue", "source", 101, 51);

    begin_app_server_fork_handoff(&path, request("guarded-unresolved")).unwrap();
    record_app_server_fork_failure(
        &path,
        "guarded-unresolved",
        "thread/fork response was interrupted",
        true,
    )
    .unwrap();

    let result = enqueue_if_mirror_matches(
        &path,
        prepared,
        ExpectedMirrorMapping {
            discord_channel_id: 101,
            target_thread_id: "source",
        },
    );
    assert!(matches!(
        result,
        Err(StoreError::ForkHandoffUnresolved {
            target_thread_id,
            last_error: Some(error),
        }) if target_thread_id == "source" && error.contains("interrupted")
    ));
    assert!(list(&path).unwrap().is_empty());
}

#[test]
fn plain_enqueue_rejects_a_completed_handoff_source_before_duplicate_read() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plain-moved-source.sqlite");
    upsert_thread(&path, "source", "project", "Source", 100, 101, 1.0).unwrap();
    enqueue(&path, job("durable-first", "source", 101, 52)).unwrap();
    let prepared_before_handoff = job("stale-new-id", "source", 101, 52);

    begin_app_server_fork_handoff(&path, request("plain-moved-source")).unwrap();
    complete_app_server_fork_handoff(&path, "plain-moved-source", "fork", 9).unwrap();

    let result = enqueue(&path, prepared_before_handoff);
    assert!(matches!(
        result,
        Err(StoreError::ForkHandoffTargetMoved {
            source_thread_id,
            target_thread_id,
        }) if source_thread_id == "source" && target_thread_id == "fork"
    ));
    let jobs = list(&path).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "durable-first");
    assert_eq!(jobs[0].target_thread_id, "fork");
}

fn request(handoff_id: &str) -> NewAppServerForkHandoff<'_> {
    NewAppServerForkHandoff {
        handoff_id,
        ambiguous_job_id: None,
        source_thread_id: "source",
        expected_generation: 4,
        quarantine_reason: "ownership fork",
    }
}

fn job<'a>(id: &'a str, target: &'a str, channel_id: i64, message_id: i64) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id,
        owner_user_id: Some(7),
        discord_message_id: Some(message_id),
        app_server_generation: 4,
        prompt: "keep",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
