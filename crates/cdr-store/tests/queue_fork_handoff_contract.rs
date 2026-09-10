use std::path::Path;
use std::process::Command;

use cdr_store::mapping::{
    MirrorDetailMode, get_detail_mode, set_detail_mode, thread_channels, upsert_thread,
};
use cdr_store::queue::{
    CompletedAppServerForkHandoff, NewAppServerForkHandoff, NewQueueJob, QueueJobState,
    begin_app_server_fork_handoff, begin_attempt, complete_app_server_fork_handoff, enqueue,
    is_app_server_managed_target, list, record_preflight_failure, record_start_failure,
    unresolved_app_server_fork_handoff_for_source,
};
use cdr_store::schema::open_initialized;

#[test]
fn fork_handoff_is_durable_atomic_and_preserves_replay_barriers() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("handoff.sqlite");
    upsert_thread(&path, "source", "project", "Original", 700, 701, 1.0)
        .expect("map source thread");
    set_detail_mode(&path, "source", MirrorDetailMode::All).expect("enable all detail mode");

    enqueue(
        &path,
        job("ambiguous", "source", 41, 101, "keep ambiguous", 1.0),
    )
    .expect("enqueue ambiguous job");
    let _ = begin_attempt(
        &path,
        "ambiguous",
        &["baseline-a".into(), "baseline-b".into()],
        41,
    )
    .expect("begin ambiguous attempt");
    let before = record_start_failure(
        &path,
        "ambiguous",
        41,
        "app-server request thread/resume timed out after 10000 ms",
        true,
    )
    .expect("record the exact ambiguous transport failure");
    enqueue(&path, job("pending-a", "source", 41, 102, "keep a", 2.0))
        .expect("enqueue first pending");
    enqueue(&path, job("pending-b", "source", 39, 103, "keep b", 3.0))
        .expect("enqueue second pending");
    record_preflight_failure(&path, "pending-a", 41, "old writer conflict")
        .expect("record first stale backoff");
    record_preflight_failure(&path, "pending-b", 39, "old writer conflict")
        .expect("record second stale backoff");
    let queued = list(&path).expect("snapshot pending payloads");
    let pending_a = find(&queued, "pending-a").clone();
    let pending_b = find(&queued, "pending-b").clone();
    enqueue(&path, job("other", "other", 9, 104, "unrelated", 4.0)).expect("enqueue unrelated job");

    let request = NewAppServerForkHandoff {
        handoff_id: "handoff-a",
        ambiguous_job_id: Some("ambiguous"),
        source_thread_id: "source",
        expected_generation: 41,
        quarantine_reason: "operator-confirmed ambiguous start",
    };
    let begun =
        begin_app_server_fork_handoff(&path, request).expect("persist intent before external fork");
    assert!(begun.created);
    assert_eq!(begun.handoff.target_thread_id, None);
    assert_eq!(
        unresolved_app_server_fork_handoff_for_source(&path, "source")
            .expect("read unresolved intent"),
        Some(begun.handoff.clone())
    );
    assert!(!is_app_server_managed_target(&path, "fork").expect("read ownership"));

    let repeated = begin_app_server_fork_handoff(&path, request).expect("same begin is idempotent");
    assert!(!repeated.created);
    assert_eq!(repeated.handoff, begun.handoff);

    let completed = complete_app_server_fork_handoff(&path, "handoff-a", "fork", 77)
        .expect("atomically complete validated fork handoff");
    assert_successful_handoff(&path, &before, &pending_a, &pending_b, &completed);
}

fn assert_successful_handoff(
    path: &Path,
    ambiguous_before: &cdr_store::queue::StoredQueueJob,
    pending_a: &cdr_store::queue::StoredQueueJob,
    pending_b: &cdr_store::queue::StoredQueueJob,
    completed: &CompletedAppServerForkHandoff,
) {
    assert!(completed.applied);
    let quarantined = completed
        .quarantined_job
        .as_ref()
        .expect("ambiguous handoff quarantines its observed job");
    assert_eq!(quarantined.state, QueueJobState::Quarantined);
    assert_preserved_ambiguous(ambiguous_before, quarantined);
    assert_eq!(completed.retargeted_jobs.len(), 2);
    assert_eq!(
        completed
            .retargeted_jobs
            .iter()
            .map(|job| job.job_id.as_str())
            .collect::<Vec<_>>(),
        vec!["pending-a", "pending-b"]
    );

    let jobs = list(path).expect("list transitioned jobs");
    let moved_a = find(&jobs, "pending-a");
    let moved_b = find(&jobs, "pending-b");
    assert_moved_pending(pending_a, moved_a, 77);
    assert_moved_pending(pending_b, moved_b, 77);
    assert_eq!(find(&jobs, "other").target_thread_id, "other");
    assert_eq!(
        thread_channels(path, "source").expect("read old mapping"),
        None
    );
    assert_eq!(
        thread_channels(path, "fork").expect("read new mapping"),
        Some((700, 701))
    );
    assert_eq!(
        get_detail_mode(path, "fork").expect("read transferred detail mode"),
        MirrorDetailMode::All
    );
    assert_eq!(
        get_detail_mode(path, "source").expect("old detail row is gone"),
        MirrorDetailMode::Send
    );
    let connection = open_initialized(path).expect("inspect mapping uniqueness");
    let mapped: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM mirror_threads WHERE discord_thread_id = 701",
            [],
            |row| row.get(0),
        )
        .expect("count exact Discord mappings");
    assert_eq!(mapped, 1);
    drop(connection);

    assert_eq!(
        unresolved_app_server_fork_handoff_for_source(path, "source")
            .expect("completed intent is no longer unresolved"),
        None
    );
    assert!(is_app_server_managed_target(path, "fork").expect("fork ownership persisted"));
    assert!(!is_app_server_managed_target(path, "source").expect("source is not managed"));

    let duplicate = enqueue(
        path,
        job("duplicate-id", "fork", 77, 101, "must not replace", 9.0),
    )
    .expect("duplicate source message resolves to durable quarantine row");
    assert!(!duplicate.created);
    assert_eq!(duplicate.job.job_id, "ambiguous");
    assert_eq!(duplicate.job.state, QueueJobState::Quarantined);

    assert_python_reads_quarantine_as_non_replayable_running(path);
}

#[test]
fn completed_handoff_is_idempotent_but_cannot_change_its_fork_target() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("idempotent.sqlite");
    upsert_thread(&path, "source", "project", "Original", 800, 801, 1.0)
        .expect("map source thread");
    enqueue(&path, job("ambiguous", "source", 5, 201, "keep", 1.0)).expect("enqueue ambiguous job");
    begin_attempt(&path, "ambiguous", &[], 5).expect("begin attempt");
    record_start_failure(&path, "ambiguous", 5, "known ambiguous start", true)
        .expect("establish ambiguity after the fresh claim lease");
    let request = NewAppServerForkHandoff {
        handoff_id: "handoff-b",
        ambiguous_job_id: Some("ambiguous"),
        source_thread_id: "source",
        expected_generation: 5,
        quarantine_reason: "ambiguous",
    };
    begin_app_server_fork_handoff(&path, request).expect("begin handoff");
    let first =
        complete_app_server_fork_handoff(&path, "handoff-b", "fork", 6).expect("complete handoff");
    assert!(first.applied);
    let second = complete_app_server_fork_handoff(&path, "handoff-b", "fork", 99)
        .expect("same completion is idempotent");
    assert!(!second.applied);
    assert_eq!(second.handoff, first.handoff);
    assert!(complete_app_server_fork_handoff(&path, "handoff-b", "different", 99).is_err());
    assert_eq!(thread_channels(&path, "fork").unwrap(), Some((800, 801)));
    assert_eq!(thread_channels(&path, "different").unwrap(), None);
}

fn job<'a>(
    id: &'a str,
    target: &'a str,
    generation: i64,
    message: i64,
    prompt: &'a str,
    created_at: f64,
) -> NewQueueJob<'a> {
    NewQueueJob {
        job_id: id,
        target_thread_id: target,
        channel_id: 701,
        owner_user_id: Some(88),
        discord_message_id: Some(message),
        app_server_generation: generation,
        prompt,
        queued: true,
        ack_sent: true,
        created_at,
    }
}

fn find<'a>(
    jobs: &'a [cdr_store::queue::StoredQueueJob],
    id: &str,
) -> &'a cdr_store::queue::StoredQueueJob {
    jobs.iter()
        .find(|job| job.job_id == id)
        .expect("job exists")
}

fn assert_preserved_ambiguous(
    before: &cdr_store::queue::StoredQueueJob,
    after: &cdr_store::queue::StoredQueueJob,
) {
    assert_eq!(after.job_id, before.job_id);
    assert_eq!(after.target_thread_id, before.target_thread_id);
    assert_eq!(after.channel_id, before.channel_id);
    assert_eq!(after.owner_user_id, before.owner_user_id);
    assert_eq!(after.discord_message_id, before.discord_message_id);
    assert_eq!(after.app_server_generation, before.app_server_generation);
    assert_eq!(after.goal_waiting, before.goal_waiting);
    assert_eq!(after.prompt, before.prompt);
    assert_eq!(after.queued, before.queued);
    assert_eq!(after.ack_sent, before.ack_sent);
    assert_eq!(after.baseline_turn_ids, before.baseline_turn_ids);
    assert_eq!(after.attempt_count, before.attempt_count);
    assert_eq!(after.created_at.to_bits(), before.created_at.to_bits());
    assert!(
        after
            .last_error
            .contains("operator-confirmed ambiguous start")
    );
    assert!(
        after
            .last_error
            .contains("app-server request thread/resume timed out after 10000 ms")
    );
}

fn assert_moved_pending(
    before: &cdr_store::queue::StoredQueueJob,
    after: &cdr_store::queue::StoredQueueJob,
    generation: i64,
) {
    assert_eq!(after.state, QueueJobState::Pending);
    assert_eq!(after.target_thread_id, "fork");
    assert_eq!(after.app_server_generation, generation);
    assert_eq!(after.job_id, before.job_id);
    assert_eq!(after.channel_id, before.channel_id);
    assert_eq!(after.owner_user_id, before.owner_user_id);
    assert_eq!(after.discord_message_id, before.discord_message_id);
    assert_eq!(after.goal_waiting, before.goal_waiting);
    assert_eq!(after.prompt, before.prompt);
    assert_eq!(after.queued, before.queued);
    assert_eq!(after.ack_sent, before.ack_sent);
    assert_eq!(after.baseline_turn_ids, before.baseline_turn_ids);
    assert_eq!(after.attempt_count, before.attempt_count);
    assert_eq!(after.turn_id, before.turn_id);
    assert_eq!(after.created_at.to_bits(), before.created_at.to_bits());
    assert!(after.last_error.is_empty());
}

fn assert_python_reads_quarantine_as_non_replayable_running(path: &Path) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "import sys; from pathlib import Path; from codex_discord_store_queue import list_queue_jobs; jobs=list_queue_jobs(Path(sys.argv[1])); match=[j for j in jobs if j.job_id == 'ambiguous']; assert len(match) == 1; assert match[0].state.value == 'running'; assert match[0].turn_id.startswith('cdr-quarantined:')";
    let output = python_command(&repo)
        .current_dir(repo)
        .args(["-c", script])
        .arg(path)
        .output()
        .expect("run Python rollback probe");
    assert!(
        output.status.success(),
        "Python rollback probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn python_command(repo: &Path) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("py");
        command.arg("-3");
        command
    } else {
        Command::new(repo.join("remote_mcp_server/.venv/bin/python"))
    }
}
