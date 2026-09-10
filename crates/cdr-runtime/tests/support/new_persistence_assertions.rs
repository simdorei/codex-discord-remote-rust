use super::*;
use std::path::Path;

pub(super) fn setup_project(state: &Path, db: &Path, cwd: &str, origin: u64) {
    let connection = rusqlite::Connection::open(state).unwrap();
    connection
        .execute_batch(include_str!("../fixtures/action_state.sql"))
        .unwrap();
    upsert_project(db, cwd, "test", 99, 1.0, |a, b| a == b).unwrap();
    if origin == 101 {
        connection
            .execute("UPDATE threads SET cwd=? WHERE id='thread-a'", [cwd])
            .unwrap();
        cdr_store::mapping::upsert_thread(db, "thread-a", cwd, "existing", 99, 101, 1.0).unwrap();
    }
}

pub(super) fn verify_rpc_counts(log: &Path, cwd: &str, fail: bool) {
    let calls = support::rpc_log(log);
    let starts = calls
        .iter()
        .filter(|c| c["method"] == "thread/start")
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["params"]["cwd"], cwd);
    assert_eq!(
        calls.iter().filter(|c| c["method"] == "turn/start").count(),
        usize::from(!fail)
    );
}

pub(super) async fn verify_persisted(
    state: &Path,
    cwd: &str,
    server: &cdr_app_server::ResidentAppServer,
) {
    let persisted = cdr_codex_state::CodexThreadStore::open(state)
        .unwrap()
        .load_thread("new-thread", false)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.cwd, cwd);
    assert!(
        std::fs::read_to_string(&persisted.rollout_path)
            .unwrap()
            .contains("새 작업")
    );
    let read = server
        .execute(
            cdr_app_server::requests::read_thread("new-thread", true),
            Some(server.generation()),
        )
        .await
        .unwrap();
    assert_eq!(read["thread"]["id"], "new-thread");
    assert_eq!(
        read["thread"]["turns"][0]["items"][0]["content"][0]["text"],
        "새 작업"
    );
}

pub(super) async fn verify_completion(db: &Path, queue: &QueueCoordinator<AppServerTurnBackend>) {
    let delivery = queue
        .stage_turn_completion("new-thread", "first-turn", "Final\n첫 답변")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivery.channel_id, 100);
    assert_eq!(delivery.target_thread_id, "new-thread");
    assert!(
        queue
            .stage_turn_completion("new-thread", "first-turn", "duplicate")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(cdr_store::delivery::list_pending(db).unwrap().len(), 1);
}
