use cdr_store::mapping::{MirrorDetailMode, get_detail_mode, thread_channels, upsert_thread};
use cdr_store::queue::{
    AppServerForkHandoffError, NewAppServerForkHandoff, NewQueueJob, begin_app_server_fork_handoff,
    completed_app_server_fork_target_for_source, enqueue, finalize_app_server_fork_handoff,
    is_app_server_managed_target, list, stage_app_server_fork_target,
    unresolved_app_server_fork_handoff_for_source,
};
use cdr_store::schema::open_initialized;

#[test]
fn unmapped_selected_thread_uses_sentinel_and_retargets_queue_without_creating_a_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unmapped.sqlite");
    enqueue(&path, job()).unwrap();
    open_initialized(&path)
        .unwrap()
        .execute(
            "INSERT INTO session_mirror_details (codex_thread_id, detail_mode) \
             VALUES ('source', 'all')",
            [],
        )
        .unwrap();

    let begun = begin_app_server_fork_handoff(&path, request("unmapped"))
        .expect("an unmapped selected Desktop thread can be fenced");
    assert_eq!(
        (
            begun.handoff.discord_channel_id,
            begun.handoff.discord_thread_id
        ),
        (0, 0)
    );
    stage_app_server_fork_target(&path, "unmapped", "fork").unwrap();
    finalize_app_server_fork_handoff(&path, "unmapped", 9).unwrap();

    assert_eq!(list(&path).unwrap()[0].target_thread_id, "fork");
    assert_eq!(thread_channels(&path, "source").unwrap(), None);
    assert_eq!(thread_channels(&path, "fork").unwrap(), None);
    assert_eq!(
        get_detail_mode(&path, "source").unwrap(),
        MirrorDetailMode::All
    );
    assert_eq!(
        get_detail_mode(&path, "fork").unwrap(),
        MirrorDetailMode::Send
    );
    assert!(is_app_server_managed_target(&path, "fork").unwrap());
    assert_eq!(
        completed_app_server_fork_target_for_source(&path, "source").unwrap(),
        Some("fork".into())
    );
}

#[test]
fn a_mapping_created_after_unmapped_begin_blocks_finalize_but_keeps_observation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mapping-appeared.sqlite");
    enqueue(&path, job()).unwrap();
    begin_app_server_fork_handoff(&path, request("appeared")).unwrap();
    stage_app_server_fork_target(&path, "appeared", "fork").unwrap();
    upsert_thread(&path, "source", "project", "Now mapped", 100, 101, 2.0).unwrap();

    assert!(matches!(
        finalize_app_server_fork_handoff(&path, "appeared", 9),
        Err(AppServerForkHandoffError::MissingOrStaleMapping { .. })
    ));
    assert_eq!(list(&path).unwrap()[0].target_thread_id, "source");
    let unresolved = unresolved_app_server_fork_handoff_for_source(&path, "source")
        .unwrap()
        .unwrap();
    assert_eq!(
        unresolved.observed_target_thread_id.as_deref(),
        Some("fork")
    );
    assert_eq!(thread_channels(&path, "source").unwrap(), Some((100, 101)));
}

fn request(handoff_id: &str) -> NewAppServerForkHandoff<'_> {
    NewAppServerForkHandoff {
        handoff_id,
        ambiguous_job_id: None,
        source_thread_id: "source",
        expected_generation: 4,
        quarantine_reason: "selected Desktop thread ownership fork",
    }
}

fn job() -> NewQueueJob<'static> {
    NewQueueJob {
        job_id: "pending",
        target_thread_id: "source",
        channel_id: 55,
        owner_user_id: Some(7),
        discord_message_id: Some(10),
        app_server_generation: 4,
        prompt: "keep",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}
