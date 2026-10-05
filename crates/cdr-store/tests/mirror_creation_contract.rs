use cdr_store::mapping::{
    MirrorThreadUpdate, commit_thread_sync,
    creation::{self, CreationScope},
    thread_channels, upsert_thread,
};
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
};

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror.sqlite");
    upsert_thread(&path, "thread-a", "alpha", "old", 20, 30, 1.0).unwrap();
    (temp, path)
}

fn scope() -> CreationScope<'static> {
    CreationScope {
        thread: "thread-a",
        guild: 1,
        parent: 20,
        expected: Some((20, 30)),
    }
}

fn update() -> MirrorThreadUpdate<'static> {
    MirrorThreadUpdate {
        thread_id: "thread-a",
        project_key: "alpha",
        title: "new",
        parent_id: 20,
        channel_id: 1001,
        now: 2.0,
    }
}

#[test]
fn ignored_intent_insert_cannot_grant_creation_ownership() {
    let (_temp, path) = fixture();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_intent BEFORE INSERT ON cdr_mirror_thread_creations
        BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    assert!(creation::begin(&path, scope()).is_err());
    assert_eq!(creation::confirmed(&path, scope()).unwrap(), None);
}

#[test]
fn confirmation_requires_exact_scope_and_token() {
    let (_temp, path) = fixture();
    let token = creation::begin(&path, scope()).unwrap();
    assert!(creation::confirm(&path, scope(), "stale-token", 1001).is_err());
    assert!(
        creation::confirm(
            &path,
            CreationScope {
                parent: 21,
                ..scope()
            },
            &token,
            1001
        )
        .is_err()
    );
    assert!(creation::confirmed(&path, scope()).is_err());
    creation::confirm(&path, scope(), &token, 1001).unwrap();
    assert_eq!(creation::confirmed(&path, scope()).unwrap(), Some(1001));
    assert!(
        creation::confirmed(
            &path,
            CreationScope {
                guild: 2,
                ..scope()
            }
        )
        .is_err()
    );
    assert!(creation::begin(&path, scope()).is_err());
}

#[test]
fn ignored_confirmation_keeps_unknown_custody() {
    let (_temp, path) = fixture();
    let token = creation::begin(&path, scope()).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_confirmation BEFORE UPDATE ON cdr_mirror_thread_creations
        BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    assert!(creation::confirm(&path, scope(), &token, 1001).is_err());
    assert!(creation::confirmed(&path, scope()).is_err());
    assert!(creation::begin(&path, scope()).is_err());
}

fn ignored_completion(trigger: &str) {
    let (_temp, path) = fixture();
    let token = creation::begin(&path, scope()).unwrap();
    creation::confirm(&path, scope(), &token, 1001).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(trigger).unwrap();
    assert!(commit_thread_sync(&path, update(), scope().expected).is_err());
    assert_eq!(thread_channels(&path, "thread-a").unwrap(), Some((20, 30)));
    assert_eq!(creation::confirmed(&path, scope()).unwrap(), Some(1001));
    db.execute_batch("DROP TRIGGER ignore_completion;").unwrap();
    commit_thread_sync(&path, update(), scope().expected).unwrap();
    assert_eq!(
        thread_channels(&path, "thread-a").unwrap(),
        Some((20, 1001))
    );
    assert_eq!(
        creation::confirmed(
            &path,
            CreationScope {
                expected: Some((20, 1001)),
                ..scope()
            }
        )
        .unwrap(),
        None
    );
    // Completed mapping does not permanently prohibit a later authoritative 404 replacement.
    creation::begin(
        &path,
        CreationScope {
            expected: Some((20, 1001)),
            ..scope()
        },
    )
    .unwrap();
}

#[test]
fn ignored_mapping_write_retains_confirmed_creation() {
    ignored_completion(
        "CREATE TRIGGER ignore_completion BEFORE UPDATE ON mirror_threads
        BEGIN SELECT RAISE(IGNORE); END;",
    );
}

#[test]
fn ignored_intent_completion_rolls_back_mapping_and_reuses_confirmed_id() {
    ignored_completion(
        "CREATE TRIGGER ignore_completion BEFORE DELETE ON cdr_mirror_thread_creations
        BEGIN SELECT RAISE(IGNORE); END;",
    );
}

#[test]
fn creation_finish_rejects_mapping_change_after_intent_delete() {
    ignored_completion("CREATE TRIGGER ignore_completion AFTER DELETE ON cdr_mirror_thread_creations
        BEGIN UPDATE mirror_threads SET discord_thread_id=777 WHERE codex_thread_id=OLD.thread_id; END;");
}

#[test]
fn creation_finish_rejects_intent_loss_during_mapping_write() {
    ignored_completion(
        "CREATE TRIGGER ignore_completion AFTER UPDATE ON mirror_threads
        WHEN NEW.discord_thread_id=1001
        BEGIN DELETE FROM cdr_mirror_thread_creations WHERE thread_id=NEW.codex_thread_id; END;",
    );
}

#[test]
fn creation_finish_rejects_owner_replacement_during_mapping_write() {
    ignored_completion("CREATE TRIGGER ignore_completion AFTER UPDATE ON mirror_threads
        WHEN NEW.discord_thread_id=1001
        BEGIN UPDATE cdr_mirror_thread_creations SET token='replaced-owner' WHERE thread_id=NEW.codex_thread_id; END;");
}

#[test]
fn pending_creation_protects_unknown_and_confirmed_rooms_from_cleanup() {
    let (_temp, path) = fixture();
    let token = creation::begin(&path, scope()).unwrap();
    assert!(cdr_store::room_cleanup::begin(&path, 30, Some("thread-a"), 2.0).is_err());
    assert!(cdr_store::room_cleanup::begin(&path, 777, None, 2.0).is_err());
    assert!(cdr_store::room_cleanup::begin(&path, 20, None, 2.0).is_err());
    creation::confirm(&path, scope(), &token, 1001).unwrap();
    assert!(cdr_store::room_cleanup::begin(&path, 1001, None, 2.0).is_err());
    // Once the exact created ID is known, unrelated orphan cleanup is not held globally.
    cdr_store::room_cleanup::begin(&path, 777, None, 2.0).unwrap();
    assert_eq!(cdr_store::room_cleanup::phase(&path, 1001).unwrap(), None);
}

#[test]
fn cleanup_and_mapping_changes_cannot_be_bypassed_by_new_claims() {
    let (_temp, path) = fixture();
    cdr_store::room_cleanup::begin(&path, 30, Some("thread-a"), 2.0).unwrap();
    assert!(creation::begin(&path, scope()).is_err());
    let (_other, path) = fixture();
    upsert_thread(&path, "thread-a", "alpha", "manual", 20, 31, 2.0).unwrap();
    assert!(creation::begin(&path, scope()).is_err());
}

#[test]
fn independent_coordinators_get_only_one_creation_claim() {
    let (_temp, path) = fixture();
    let barrier = Arc::new(Barrier::new(3));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let barrier = barrier.clone();
        tasks.push(std::thread::spawn(move || {
            barrier.wait();
            creation::begin(&path, scope()).is_ok()
        }));
    }
    barrier.wait();
    assert_eq!(
        tasks
            .into_iter()
            .filter(|task| task.thread().id() != std::thread::current().id())
            .map(|task| usize::from(task.join().unwrap()))
            .sum::<usize>(),
        1
    );
}
