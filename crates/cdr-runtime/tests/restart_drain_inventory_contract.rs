use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{
    NewAppServerForkHandoff, begin_app_server_fork_handoff, stage_app_server_fork_target,
};

#[test]
fn unresolved_fork_source_and_observed_destination_are_in_live_drain_inventory() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    upsert_thread(&db, "fork-source", "project", "Source", 70, 71, 1.0).unwrap();
    begin_app_server_fork_handoff(
        &db,
        NewAppServerForkHandoff {
            handoff_id: "handoff-a",
            ambiguous_job_id: None,
            source_thread_id: "fork-source",
            expected_generation: 1,
            quarantine_reason: "test inventory",
        },
    )
    .unwrap();
    stage_app_server_fork_target(&db, "handoff-a", "fork-observed").unwrap();

    let snapshot = cdr_store::restart_readiness::snapshot(&db).unwrap();
    assert!(snapshot.target_thread_ids.contains("fork-source"));
    assert!(snapshot.target_thread_ids.contains("fork-observed"));
    assert!(
        snapshot
            .observations
            .iter()
            .any(|value| value.contains("handoff-a"))
    );
}
