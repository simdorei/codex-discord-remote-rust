use super::*;
use cdr_store::mirror::{get_cursor_turn, get_offset, update_cursor_with_turn};

pub(super) fn history(id: &str) -> Vec<u8> {
    format!(
        "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\"}}}}\n\
         {{\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"phase\":\"commentary\",\"message\":\"기존 답변\"}}}}\n"
    ).into_bytes()
}

pub(super) fn archive(temp: &tempfile::TempDir, bytes: &[u8]) -> Vec<u8> {
    let compressed = zstd::stream::encode_all(bytes, 1).unwrap();
    fs::write(temp.path().join("rollout.jsonl.zst"), &compressed).unwrap();
    fs::remove_file(temp.path().join("rollout.jsonl")).unwrap();
    compressed
}

pub(super) fn save_cursor(temp: &tempfile::TempDir, cursor: i64) {
    update_cursor_with_turn(
        &temp.path().join("mirror.sqlite"),
        "thread-a",
        temp.path().join("rollout.jsonl").to_str().unwrap(),
        cursor,
        42.0,
        Some("existing-turn"),
    )
    .unwrap();
}

#[tokio::test]
async fn compressed_sync_preserves_valid_cursor_context_and_existing_room_on_retry() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, remote) = fixture(&temp);
    let bytes = history("thread-a");
    let position = i64::try_from(bytes.iter().position(|b| *b == b'\n').unwrap() + 1).unwrap();
    save_cursor(&temp, position);
    let db = temp.path().join("mirror.sqlite");
    let before = get_offset(&db, "thread-a").unwrap();
    let compressed = archive(&temp, &bytes);
    for _ in 0..2 {
        let result = sync.sync(99, Some(1)).await.unwrap();
        assert_eq!((result.threads, result.unavailable), (1, 0));
        assert_eq!(fs::read(temp.path().join("rollout.jsonl")).unwrap(), bytes);
        assert_eq!(
            fs::read(temp.path().join("rollout.jsonl.zst")).unwrap(),
            compressed
        );
        assert_eq!(get_offset(&db, "thread-a").unwrap(), before);
        assert_eq!(
            get_cursor_turn(&db, "thread-a").unwrap(),
            Some("existing-turn".into())
        );
        assert_eq!(thread_channels(&db, "thread-a").unwrap(), Some((20, 30)));
    }
    assert_eq!(remote.0.lock().unwrap().creates, 0);
    assert_eq!(remote.0.lock().unwrap().deletes, 0);
    // A subsequent compression can be recovered with the same manual sync.
    fs::remove_file(temp.path().join("rollout.jsonl")).unwrap();
    assert_eq!(sync.sync(99, Some(1)).await.unwrap().unavailable, 0);
    assert_eq!(get_offset(&db, "thread-a").unwrap(), before);
}

#[tokio::test]
async fn invalid_cursor_stops_sync_without_publishing_or_resetting_it() {
    for compressed in [false, true] {
        for cursor in [-1, 1, 999_999] {
            let temp = tempfile::tempdir().unwrap();
            let (sync, remote) = fixture(&temp);
            let bytes = history("thread-a");
            fs::write(temp.path().join("rollout.jsonl"), &bytes).unwrap();
            if compressed {
                archive(&temp, &bytes);
            }
            save_cursor(&temp, cursor);
            let db = temp.path().join("mirror.sqlite");
            let before = get_offset(&db, "thread-a").unwrap();
            let error = sync.sync(99, Some(1)).await.unwrap_err().to_string();
            assert!(
                error.contains("thread-a") && error.contains("cursor"),
                "{error}"
            );
            assert!(error.contains("explicit recovery"), "{error}");
            assert_eq!(get_offset(&db, "thread-a").unwrap(), before);
            assert_eq!(
                get_cursor_turn(&db, "thread-a").unwrap(),
                Some("existing-turn".into())
            );
            assert_eq!(temp.path().join("rollout.jsonl").exists(), !compressed);
            assert_eq!(thread_channels(&db, "thread-a").unwrap(), Some((20, 30)));
            assert_eq!(remote.0.lock().unwrap().creates, 0);
            assert_eq!(remote.0.lock().unwrap().deletes, 0);
        }
    }
}

#[tokio::test]
async fn corrupt_or_wrong_session_archive_is_explicit_and_never_published() {
    for variant in ["truncated", "corrupt", "wrong-session", "partial-record"] {
        let temp = tempfile::tempdir().unwrap();
        let (sync, remote) = fixture(&temp);
        let mut bytes = history(if variant == "wrong-session" {
            "other-thread"
        } else {
            "thread-a"
        });
        if variant == "partial-record" {
            bytes.pop();
        }
        let mut compressed = archive(&temp, &bytes);
        if variant == "truncated" {
            compressed.truncate(compressed.len() / 2);
        }
        if variant == "corrupt" {
            compressed[0] = 0;
        }
        fs::write(temp.path().join("rollout.jsonl.zst"), &compressed).unwrap();
        let error = sync.sync(99, Some(1)).await.unwrap_err().to_string();
        assert!(
            error.contains("thread-a") && error.contains("rollout"),
            "{error}"
        );
        assert!(!temp.path().join("rollout.jsonl").exists());
        assert_eq!(
            fs::read(temp.path().join("rollout.jsonl.zst")).unwrap(),
            compressed
        );
        assert_eq!(
            get_offset(&temp.path().join("mirror.sqlite"), "thread-a").unwrap(),
            None
        );
        assert_eq!(remote.0.lock().unwrap().creates, 0);
        assert_eq!(remote.0.lock().unwrap().deletes, 0);
    }
}

#[tokio::test]
async fn existing_plain_log_wins_over_bad_archive_and_missing_both_stays_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, _) = fixture(&temp);
    let bytes = history("thread-a");
    fs::write(temp.path().join("rollout.jsonl"), &bytes).unwrap();
    fs::write(temp.path().join("rollout.jsonl.zst"), b"not zstd").unwrap();
    save_cursor(&temp, i64::try_from(bytes.len()).unwrap());
    let before = get_offset(&temp.path().join("mirror.sqlite"), "thread-a").unwrap();
    assert_eq!(sync.sync(99, Some(1)).await.unwrap().unavailable, 0);
    assert_eq!(fs::read(temp.path().join("rollout.jsonl")).unwrap(), bytes);
    assert_eq!(
        get_offset(&temp.path().join("mirror.sqlite"), "thread-a").unwrap(),
        before
    );
    fs::remove_file(temp.path().join("rollout.jsonl")).unwrap();
    fs::remove_file(temp.path().join("rollout.jsonl.zst")).unwrap();
    assert_eq!(sync.sync(99, Some(1)).await.unwrap().unavailable, 1);
}

#[tokio::test]
async fn restored_untracked_log_leaves_normal_first_poll_initialization_intact() {
    let temp = tempfile::tempdir().unwrap();
    let (sync, _) = fixture(&temp);
    archive(&temp, &history("thread-a"));
    assert_eq!(sync.sync(99, Some(1)).await.unwrap().unavailable, 0);
    assert_eq!(
        get_offset(&temp.path().join("mirror.sqlite"), "thread-a").unwrap(),
        None
    );
}
