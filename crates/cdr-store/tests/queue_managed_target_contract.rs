use cdr_store::queue::{
    completed_app_server_fork_target_for_source, is_app_server_managed_target,
    mark_app_server_managed_target, unresolved_app_server_fork_handoff_for_source,
};
use cdr_store::schema::open_initialized;

#[test]
fn a_direct_app_server_thread_is_managed_without_a_synthetic_fork_cycle() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("managed-target.sqlite");
    assert!(!is_app_server_managed_target(&path, "new-thread").unwrap());

    mark_app_server_managed_target(&path, "new-thread", 4).unwrap();
    assert!(is_app_server_managed_target(&path, "new-thread").unwrap());
    assert!(
        unresolved_app_server_fork_handoff_for_source(&path, "new-thread")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        completed_app_server_fork_target_for_source(&path, "new-thread").unwrap(),
        None
    );

    mark_app_server_managed_target(&path, "new-thread", 9).unwrap();
    let connection = open_initialized(&path).unwrap();
    let (generation, created_at, updated_at): (i64, f64, f64) = connection
        .query_row(
            "SELECT app_server_generation, created_at, updated_at \
             FROM codex_app_server_managed_targets WHERE thread_id = 'new-thread'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(generation, 9);
    assert!(updated_at >= created_at);
}
