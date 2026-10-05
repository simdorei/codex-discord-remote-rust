use super::{Effect, Scope, activate, certify, discover, scope_verified};
use rusqlite::Connection;

#[test]
fn patch06_review_r1_unsealed_previous_owner_is_not_empty_coverage() {
    let (_temp, path, scope) = fixture();
    super::mark_unknown(&path, &scope, "old unsealed loss").unwrap();
    let replacement = Scope {
        owner_id: "new-owner".into(),
        generation: 1,
    };
    activate(&path, &replacement).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM cdr_observation_gaps WHERE first_seq=0", [])
        .unwrap();
    assert!(
        !scope_verified(&path, &replacement, 0).unwrap(),
        "an old unsealed stream is independent evidence, even if its marker is missing"
    );
}

#[test]
fn patch06_review_r1_ignored_unknown_insert_returns_error() {
    let (_temp, path, scope) = fixture();
    discover(&path, &scope, 1).unwrap();
    assert!(certify(&path, &scope, 1, &[Effect::NoRequiredStore]).unwrap());
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_unknown BEFORE INSERT ON cdr_observation_gaps
        WHEN NEW.first_seq=0 BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let result = super::mark_unknown(&path, &scope, "unattributed loss");
    assert!(
        result.is_err(),
        "required unknown evidence was not stored: {result:?}"
    );
}

#[test]
fn patch06_review_r1_ignored_previous_tail_rolls_back_activation() {
    let (_temp, path, scope) = fixture();
    discover(&path, &scope, 1).unwrap();
    assert!(certify(&path, &scope, 1, &[Effect::NoRequiredStore]).unwrap());
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_old_tail BEFORE INSERT ON cdr_observation_gaps
        WHEN NEW.first_seq=0 BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let replacement = Scope {
        owner_id: "replacement-owner".into(),
        generation: 1,
    };
    let result = activate(&path, &replacement);
    let old_active: bool = db
        .query_row(
            "SELECT active FROM cdr_observation_streams WHERE owner_id=?1 AND generation=?2",
            rusqlite::params![scope.owner_id, scope.generation],
            |r| r.get(0),
        )
        .unwrap();
    let new_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM cdr_observation_streams WHERE owner_id=?1",
            [&replacement.owner_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        result.is_err() && old_active && new_count == 0,
        "missing old tail must rollback activation: {result:?}, old_active={old_active}, new_count={new_count}"
    );
    assert!(!scope_verified(&path, &replacement, 0).unwrap());
}

#[test]
fn patch06_review_r1_duplicate_unknown_preserves_exact_original_evidence() {
    let (_temp, path, scope) = fixture();
    super::mark_unknown(&path, &scope, "first loss").unwrap();
    let db = Connection::open(&path).unwrap();
    let read = || {
        db.query_row(
            "SELECT gap_id,revision,detail,state FROM cdr_observation_gaps
         WHERE owner_id=?1 AND generation=?2 AND first_seq=0 AND last_seq=0",
            rusqlite::params![scope.owner_id, scope.generation],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .unwrap()
    };
    let original = read();
    super::mark_unknown(&path, &scope, "later duplicate must not rewrite evidence").unwrap();
    assert_eq!(read(), original);
    assert_eq!(original.2, "first loss");
    assert_eq!(original.3, "Unresolved");
    let replacement = Scope {
        owner_id: "next-owner".into(),
        generation: 1,
    };
    activate(&path, &replacement).unwrap();
    assert_eq!(read(), original);
    assert!(!scope_verified(&path, &replacement, 0).unwrap());
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Scope) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("review.sqlite");
    let scope = Scope {
        owner_id: "review-owner".into(),
        generation: 1,
    };
    activate(&path, &scope).unwrap();
    (temp, path, scope)
}

#[test]
fn patch06_review_p1_ignored_range_insert_cannot_advance_seen_or_clear_gap() {
    let (_temp, path, scope) = fixture();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_range BEFORE INSERT ON cdr_observation_gaps
        WHEN NEW.first_seq>0 BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let result = discover(&path, &scope, 1);
    let seen: i64 = db
        .query_row("SELECT seen_seq FROM cdr_observation_streams", [], |r| {
            r.get(0)
        })
        .unwrap();
    let verified = scope_verified(&path, &scope, 1).unwrap();
    assert!(
        result.is_err() && seen == 0 && !verified,
        "ignored required INSERT must rollback: result={result:?}, seen={seen}, verified={verified}"
    );
}

#[test]
fn patch06_review_p1_ignored_stream_update_rolls_back_new_range() {
    let (_temp, path, scope) = fixture();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER ignore_seen BEFORE UPDATE OF seen_seq ON cdr_observation_streams
        BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let result = discover(&path, &scope, 1);
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM cdr_observation_gaps", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        result.is_err() && count == 0,
        "ignored checkpoint UPDATE must rollback its range: {result:?}, ranges={count}"
    );
}

#[test]
fn patch06_review_p1_missing_verified_range_is_not_vacuous_coverage() {
    let (_temp, path, scope) = fixture();
    discover(&path, &scope, 1).unwrap();
    assert!(certify(&path, &scope, 1, &[Effect::NoRequiredStore]).unwrap());
    assert!(scope_verified(&path, &scope, 1).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM cdr_observation_gaps WHERE first_seq=1", [])
        .unwrap();
    assert!(
        !scope_verified(&path, &scope, 1).unwrap(),
        "seen watermark without stored positive coverage is not an idle certificate"
    );
}
