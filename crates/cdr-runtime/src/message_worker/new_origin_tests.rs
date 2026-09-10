use super::*;
use crate::{
    action_executor::ActionContext,
    test_support::{app_fixture, new_reply_fixture},
};
use std::sync::atomic::Ordering;

async fn reassigned_origin(after_admission: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, gate) = new_reply_fixture::setup(&temp).await;
    let db = fixture.executor.mirror_db();
    let candidate = fixture.classify_id("!new first", 801);
    let mut admitted = None;
    let mut candidate = Some(candidate);
    if after_admission {
        admitted =
            admit_message_candidate_at(candidate.take().unwrap(), std::time::SystemTime::now())
                .unwrap();
    }
    let other = temp.path().join("other-project");
    std::fs::create_dir(&other).unwrap();
    let other = other.to_string_lossy().into_owned();
    rusqlite::Connection::open(temp.path().join("state.sqlite"))
        .unwrap()
        .execute("UPDATE threads SET cwd=? WHERE id='thread-a'", [&other])
        .unwrap();
    cdr_store::schema::open_initialized(db).unwrap().execute(
        "UPDATE mirror_threads SET codex_thread_id='thread-a',project_key=? WHERE discord_thread_id=42",[&other]).unwrap();
    if let Some(candidate) = candidate {
        admitted = admit_message_candidate_at(candidate, std::time::SystemTime::now()).unwrap();
    }
    let result = fixture
        .executor
        .execute_with_context(
            CommandAction::New {
                prompt: "first".into(),
            },
            ActionContext {
                channel_id: 42,
                user_id: 3,
                discord_message_id: Some(801),
                auto_queue_when_busy: true,
            },
        )
        .await;
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "thread/start").count(),
        0,
        "origin changed before its creation context was frozen: {result:?}"
    );
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "turn/start").count(),
        0
    );
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    assert!(posts.is_empty());
    assert!(result.is_err());
    drop(admitted);
}

#[tokio::test]
async fn new_origin_changed_between_classification_and_admission_starts_nothing() {
    reassigned_origin(false).await;
}

#[tokio::test]
async fn new_origin_changed_between_admission_and_context_freeze_starts_nothing() {
    reassigned_origin(true).await;
}

#[tokio::test]
async fn codex_cwd_changed_without_mirror_sync_never_starts_in_another_project() {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, gate) = new_reply_fixture::setup(&temp).await;
    let admitted = fixture.admit("!new first");
    let other = temp.path().join("other-project");
    std::fs::create_dir(&other).unwrap();
    // Codex's index can be newer than the Discord mirror. A stable mirror row
    // alone must not authorize starting a conversation in a different folder.
    rusqlite::Connection::open(temp.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "UPDATE threads SET cwd=? WHERE id='thread-b'",
            [other.to_string_lossy().as_ref()],
        )
        .unwrap();
    let result = fixture
        .executor
        .execute_with_context(
            CommandAction::New {
                prompt: "first".into(),
            },
            ActionContext {
                channel_id: 42,
                user_id: 3,
                discord_message_id: Some(801),
                auto_queue_when_busy: true,
            },
        )
        .await;
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "thread/start").count(),
        0,
        "Codex cwd changed outside the frozen Discord project: {result:?}"
    );
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "turn/start").count(),
        0
    );
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    assert!(posts.is_empty());
    assert!(result.is_err());
    drop(admitted);
}
