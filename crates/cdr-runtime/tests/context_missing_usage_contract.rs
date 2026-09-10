use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::sync::Arc;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn missing_cumulative_usage_remains_unknown_but_recorded_zero_remains_zero() {
    for (stored, expected) in [
        (None, "미확인"),
        (Some(-1_i64), "미확인"),
        (Some(0_i64), "0"),
        (Some(123_i64), "123"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
        let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
        let backend = Arc::new(target::FakeBackend::default());
        let executor = target::executor(&root, db, bridge.clone(), backend.clone());
        let path = root.path().join("b.jsonl");
        std::fs::write(&path, concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-b\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":80000}}}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"phase\":\"final\",\"message\":\"유효한 최근 답변\"}}\n"
        )).unwrap();
        let connection = rusqlite::Connection::open(root.path().join("state.sqlite")).unwrap();
        connection
            .execute(
                "UPDATE threads SET tokens_used=?1,rollout_path=?2 WHERE id='thread-b'",
                rusqlite::params![stored, path.to_str().unwrap()],
            )
            .unwrap();
        for (all_threads, refresh) in [(false, false), (false, true), (true, false), (true, true)] {
            let text = executor
                .execute(
                    CommandAction::Context {
                        all_threads,
                        refresh,
                        limit: 10,
                    },
                    42,
                    3,
                )
                .await
                .unwrap()
                .text;
            assert!(
                text.contains(&format!("used: {expected} · 누적")),
                "stored={stored:?}, all={all_threads}: {text}"
            );
            assert!(text.contains("last_input: 80000"));
            assert_eq!(text.contains("유효한 최근 답변"), refresh);
        }
        let status = executor
            .execute(CommandAction::Status { reference: None }, 42, 3)
            .await
            .unwrap()
            .text;
        assert!(
            status.contains(&format!("tokens_used: {expected} (누적")),
            "{status}"
        );
        let list = executor
            .execute(CommandAction::List { limit: 10 }, 42, 3)
            .await
            .unwrap()
            .text;
        let row = list.lines().find(|line| line.contains("thread-b")).unwrap();
        let list_expected = match stored {
            Some(0) => "0.000K",
            Some(123) => "0.123K",
            _ => "미확인",
        };
        assert!(
            row.contains(&format!("used {list_expected} (누적)")),
            "{row}"
        );
        if expected == "미확인" {
            assert!(row.contains("rec 미확인"), "{row}");
        }
        let after: Option<i64> = connection
            .query_row(
                "SELECT tokens_used FROM threads WHERE id='thread-b'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, stored);
        assert!(!bridge.path().exists());
        assert!(
            backend.starts.lock().await.is_empty()
                && backend.resumes.lock().await.is_empty()
                && backend.forks.lock().await.is_empty()
        );
    }
}
