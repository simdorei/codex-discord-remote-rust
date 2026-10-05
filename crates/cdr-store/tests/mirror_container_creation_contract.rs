use cdr_store::mapping::{
    container_creation::{self as custody, ProjectSnapshot, Receipt},
    find_project, upsert_project, upsert_thread,
};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
};

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror.sqlite");
    upsert_project(&path, "ALPHA", "alpha", 20, 1.0, str::eq_ignore_ascii_case).unwrap();
    upsert_thread(&path, "thread-a", "ALPHA", "old", 20, 30, 1.0).unwrap();
    (temp, path)
}

fn snapshot(path: &Path) -> ProjectSnapshot {
    custody::project_snapshot(path, "alpha", str::eq_ignore_ascii_case).unwrap()
}

fn begin(path: &Path) -> Receipt {
    custody::begin_project(
        path,
        "alpha",
        1,
        10,
        &snapshot(path),
        str::eq_ignore_ascii_case,
    )
    .unwrap()
}

fn pending(path: &Path) -> cdr_store::Result<Option<Receipt>> {
    custody::project_receipt(
        path,
        "alpha",
        1,
        10,
        &snapshot(path),
        str::eq_ignore_ascii_case,
    )
}

fn mapped(path: &Path) -> i64 {
    find_project(path, Some("alpha"), str::eq_ignore_ascii_case)
        .unwrap()
        .unwrap()
        .channel_id
}

fn finish(path: &Path) -> cdr_store::Result<Vec<String>> {
    upsert_project(path, "alpha", "alpha", 1001, 2.0, str::eq_ignore_ascii_case)
}

#[test]
fn container_ignored_intent_insert_never_authorizes_create() {
    let (_temp, path) = fixture();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER ignore_intent BEFORE INSERT ON cdr_mirror_container_creations
         BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    assert!(
        custody::begin_project(
            &path,
            "alpha",
            1,
            10,
            &snapshot(&path),
            str::eq_ignore_ascii_case
        )
        .is_err()
    );
    assert_eq!(pending(&path).unwrap(), None);
    assert!(custody::begin_category(&path, 1, None).is_err());
}

#[test]
fn container_ignored_confirmation_stays_unknown() {
    let (_temp, path) = fixture();
    let receipt = begin(&path);
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER ignore_confirm BEFORE UPDATE ON cdr_mirror_container_creations
         BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    assert!(custody::confirm(&path, &receipt, 1001).is_err());
    assert!(pending(&path).is_err());
    assert!(finish(&path).is_err());
    assert_eq!(mapped(&path), 20);
}

fn atomic_completion(trigger: &str) {
    let (_temp, path) = fixture();
    let receipt = custody::confirm(&path, &begin(&path), 1001).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(trigger).unwrap();
    assert!(
        finish(&path).is_err(),
        "mapping, custody and final checks must commit together"
    );
    assert_eq!(mapped(&path), 20);
    assert_eq!(snapshot(&path), vec![("ALPHA".into(), 20)]);
    assert_eq!(pending(&path).unwrap(), Some(receipt));
    let project: String = db
        .query_row(
            "SELECT project_key FROM mirror_threads WHERE codex_thread_id='thread-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        project, "ALPHA",
        "alias normalization rolls back with the mapping"
    );
    db.execute_batch("DROP TRIGGER break_completion;").unwrap();
    finish(&path).unwrap();
    assert_eq!(mapped(&path), 1001);
    assert_eq!(pending(&path).unwrap(), None);
    // A new authoritative missing-room lookup can begin a later, independent episode.
    begin(&path);
}

#[test]
fn container_ignored_project_mapping_rolls_back_aliases() {
    atomic_completion(
        "CREATE TRIGGER break_completion BEFORE INSERT ON mirror_projects
        BEGIN SELECT RAISE(IGNORE); END;",
    );
}

#[test]
fn container_ignored_receipt_delete_rolls_back_mapping() {
    atomic_completion(
        "CREATE TRIGGER break_completion BEFORE DELETE ON cdr_mirror_container_creations
        BEGIN SELECT RAISE(IGNORE); END;",
    );
}

#[test]
fn container_mapping_trigger_cannot_remove_custody() {
    atomic_completion(
        "CREATE TRIGGER break_completion AFTER INSERT ON mirror_projects
        BEGIN DELETE FROM cdr_mirror_container_creations WHERE kind='project'; END;",
    );
}

#[test]
fn container_mapping_trigger_cannot_replace_custody() {
    atomic_completion(
        "CREATE TRIGGER break_completion AFTER INSERT ON mirror_projects
        BEGIN UPDATE cdr_mirror_container_creations SET token='changed' WHERE kind='project'; END;",
    );
}

#[test]
fn container_receipt_delete_cannot_change_final_mapping() {
    atomic_completion(
        "CREATE TRIGGER break_completion AFTER DELETE ON cdr_mirror_container_creations
        BEGIN UPDATE mirror_projects SET discord_channel_id=777; END;",
    );
}

#[test]
fn container_receipt_delete_cannot_add_cleanup_fence() {
    atomic_completion(
        "CREATE TRIGGER break_completion AFTER DELETE ON cdr_mirror_container_creations
        BEGIN INSERT INTO cdr_cleanup_fences(channel_id,target_thread_id,token,phase,created_at)
        VALUES(1001,NULL,'late-cleanup','deleting',2); END;",
    );
}

#[test]
fn container_alias_insert_cannot_hide_conflicting_mapping() {
    atomic_completion("CREATE TRIGGER break_completion AFTER INSERT ON mirror_projects WHEN NEW.project_key='alpha'
        BEGIN INSERT INTO mirror_projects VALUES('Alpha','alias',777,2); END;");
}

#[test]
fn container_cleanup_and_other_project_adoption_are_guarded() {
    let (_temp, path) = fixture();
    upsert_project(&path, "beta", "beta", 50, 1.0, str::eq_ignore_ascii_case).unwrap();
    upsert_thread(&path, "thread-b", "beta", "other", 50, 60, 1.0).unwrap();
    let receipt = begin(&path);
    assert!(cdr_store::room_cleanup::begin(&path, 30, Some("thread-a"), 2.0).is_err());
    assert!(cdr_store::room_cleanup::begin(&path, 777, None, 2.0).is_err());
    assert!(custody::ensure_adoptable(&path, 1001, 10).is_err());
    assert!(
        upsert_project(
            &path,
            "gamma",
            "gamma",
            1001,
            2.0,
            str::eq_ignore_ascii_case
        )
        .is_err()
    );
    // A mapped unrelated target is not globally blocked by A's uncertainty.
    upsert_project(&path, "beta", "beta", 50, 2.0, str::eq_ignore_ascii_case).unwrap();
    cdr_store::room_cleanup::begin(&path, 60, Some("thread-b"), 2.0).unwrap();
    custody::confirm(&path, &receipt, 1001).unwrap();
    assert!(custody::ensure_adoptable(&path, 1001, 10).is_err());
    assert!(cdr_store::room_cleanup::begin(&path, 1001, None, 2.0).is_err());
    cdr_store::room_cleanup::begin(&path, 777, None, 2.0).unwrap();
    finish(&path).unwrap();
}

#[test]
fn container_scope_and_mapping_changes_never_authorize_recreation() {
    let (_temp, path) = fixture();
    let receipt = begin(&path);
    custody::confirm(&path, &receipt, 1001).unwrap();
    assert!(
        custody::project_receipt(
            &path,
            "alpha",
            2,
            10,
            &snapshot(&path),
            str::eq_ignore_ascii_case
        )
        .is_err()
    );
    assert!(
        custody::project_receipt(
            &path,
            "alpha",
            1,
            11,
            &snapshot(&path),
            str::eq_ignore_ascii_case
        )
        .is_err()
    );
    let original = snapshot(&path);
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE mirror_projects SET discord_channel_id=22", [])
        .unwrap();
    assert!(
        custody::project_receipt(&path, "alpha", 1, 10, &original, str::eq_ignore_ascii_case)
            .is_err()
    );
    assert!(pending(&path).is_err());
    assert!(
        custody::begin_project(
            &path,
            "alpha",
            1,
            10,
            &snapshot(&path),
            str::eq_ignore_ascii_case
        )
        .is_err()
    );
    assert!(finish(&path).is_err());
    assert_eq!(mapped(&path), 22);
}

#[test]
fn container_new_claim_respects_existing_lifecycle_fence() {
    let (_temp, path) = fixture();
    cdr_store::room_cleanup::begin(&path, 30, Some("thread-a"), 2.0).unwrap();
    assert!(
        custody::begin_project(
            &path,
            "alpha",
            1,
            10,
            &snapshot(&path),
            str::eq_ignore_ascii_case
        )
        .is_err()
    );
    assert_eq!(pending(&path).unwrap(), None);
}

#[test]
fn container_category_binding_and_fresh_missing_replacement_keep_exact_token() {
    let (_temp, path) = fixture();
    let attempted = custody::begin_category(&path, 1, None).unwrap();
    assert!(custody::category_receipt(&path, 1).is_err());
    assert!(custody::begin_category(&path, 1, None).is_err());
    let confirmed = custody::confirm(&path, &attempted, 1001).unwrap();
    assert!(custody::begin_category(&path, 1, Some(&confirmed)).is_err());
    custody::bind_category(&path, &confirmed).unwrap();
    let bound = custody::category_receipt(&path, 1).unwrap().unwrap();
    assert!(bound.is_bound());
    assert_eq!(bound.confirmed_channel().unwrap(), 1001);
    let replacement = custody::begin_category(&path, 1, Some(&bound)).unwrap();
    assert!(custody::confirm(&path, &attempted, 1002).is_err());
    assert!(custody::bind_category(&path, &bound).is_err());
    let new = custody::confirm(&path, &replacement, 1002).unwrap();
    custody::bind_category(&path, &new).unwrap();
    assert_eq!(
        custody::category_receipt(&path, 1)
            .unwrap()
            .unwrap()
            .confirmed_channel()
            .unwrap(),
        1002
    );
}

#[test]
fn container_failed_category_binding_preserves_confirmation() {
    let (_temp, path) = fixture();
    let receipt = custody::confirm(
        &path,
        &custody::begin_category(&path, 1, None).unwrap(),
        1001,
    )
    .unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_binding BEFORE UPDATE ON cdr_mirror_container_creations
        WHEN NEW.phase='bound' BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    assert!(custody::bind_category(&path, &receipt).is_err());
    assert_eq!(
        custody::category_receipt(&path, 1).unwrap(),
        Some(receipt.clone())
    );
    db.execute_batch("DROP TRIGGER ignore_binding;").unwrap();
    custody::bind_category(&path, &receipt).unwrap();
}

#[test]
fn container_independent_coordinators_get_one_project_claim() {
    let (_temp, path) = fixture();
    let expected = snapshot(&path);
    let barrier = Arc::new(Barrier::new(3));
    let tasks = (0..2)
        .map(|_| {
            let path = path.clone();
            let expected = expected.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                custody::begin_project(&path, "alpha", 1, 10, &expected, str::eq_ignore_ascii_case)
                    .is_ok()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    assert_eq!(
        tasks
            .into_iter()
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>(),
        1
    );
}

#[test]
fn container_another_confirmed_project_can_finish_without_releasing_unknown_a() {
    let (_temp, path) = fixture();
    let _a = begin(&path);
    let b =
        custody::begin_project(&path, "beta", 1, 10, &vec![], str::eq_ignore_ascii_case).unwrap();
    custody::confirm(&path, &b, 1002).unwrap();
    upsert_project(&path, "beta", "beta", 1002, 2.0, str::eq_ignore_ascii_case).unwrap();
    assert!(pending(&path).is_err());
    assert_eq!(mapped(&path), 20);
}

#[test]
fn container_claim_post_insert_mapping_change_rolls_back_before_authority() {
    let (_temp, path) = fixture();
    let original = snapshot(&path);
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER change_project_during_claim
        AFTER INSERT ON cdr_mirror_container_creations WHEN NEW.kind='project'
        BEGIN UPDATE mirror_projects SET discord_channel_id=22 WHERE project_key='ALPHA'; END;",
    )
    .unwrap();
    let result =
        custody::begin_project(&path, "alpha", 1, 10, &original, str::eq_ignore_ascii_case);
    assert!(
        result.is_err(),
        "changed snapshot cannot grant remote create authority"
    );
    assert_eq!(
        mapped(&path),
        20,
        "mapping trigger must roll back with intent"
    );
    assert_eq!(pending(&path).unwrap(), None);
    db.execute_batch("DROP TRIGGER change_project_during_claim;")
        .unwrap();
    let receipt = begin(&path);
    custody::confirm(&path, &receipt, 1001).unwrap();
    finish(&path).unwrap();
    assert_eq!(mapped(&path), 1001);
}

#[test]
fn container_claim_post_insert_alias_claim_change_rolls_back_before_authority() {
    let (_temp, path) = fixture();
    let original = snapshot(&path);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER add_alias_during_claim
        AFTER INSERT ON cdr_mirror_container_creations WHEN NEW.kind='project' AND NEW.scope_key='alpha'
        BEGIN INSERT INTO cdr_mirror_container_creations
        (kind,scope_key,token,guild_id,parent_id,expected_json,phase,channel_id)
        VALUES('project','ALPHA','alias-token',1,10,NEW.expected_json,'attempted',NULL); END;").unwrap();
    let result =
        custody::begin_project(&path, "alpha", 1, 10, &original, str::eq_ignore_ascii_case);
    assert!(
        result.is_err(),
        "a new alias claim cannot share remote create authority"
    );
    assert_eq!(snapshot(&path), original);
    assert_eq!(pending(&path).unwrap(), None);
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM cdr_mirror_container_creations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "both attempted claims must roll back");
}
