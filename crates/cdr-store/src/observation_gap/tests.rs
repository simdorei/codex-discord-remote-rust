use super::*;
fn setup() -> (tempfile::TempDir, std::path::PathBuf, Scope) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    let scope = Scope {
        owner_id: "original".into(),
        generation: 7,
    };
    activate(&path, &scope).unwrap();
    (temp, path, scope)
}
fn prove(path: &Path, scope: &Scope, first: i64, last: i64) {
    for seq in first..=last {
        assert!(certify(path, scope, seq, &[Effect::NoRequiredStore]).unwrap());
    }
}
#[test]
fn fixed_g1_progresses_after_middle_hole_while_g2_grows() {
    let (_temp, path, scope) = setup();
    discover(&path, &scope, 96).unwrap();
    let g1 = next(&path, &scope).unwrap().unwrap();
    prove(&path, &scope, 1, 32);
    assert!(
        !finish_page(&path, &g1, 32).unwrap(),
        "stale exact claim rejected"
    );
    let current = next(&path, &scope).unwrap().unwrap();
    assert!(finish_page(&path, &current, 32).unwrap());
    discover(&path, &scope, 160).unwrap();
    let current = next(&path, &scope).unwrap().unwrap();
    assert_eq!(
        (current.id, current.first, current.last, current.cursor),
        (g1.id, 1, 96, 32)
    );
    prove(&path, &scope, 41, 64);
    let current = next(&path, &scope).unwrap().unwrap();
    assert!(finish_page(&path, &current, 64).unwrap());
    discover(&path, &scope, 224).unwrap();
    prove(&path, &scope, 65, 96);
    let current = next(&path, &scope).unwrap().unwrap();
    assert_eq!(
        current.verified,
        vec![
            Span { first: 1, last: 32 },
            Span {
                first: 41,
                last: 96
            }
        ]
    );
    assert!(finish_page(&path, &current, 96).unwrap());
    assert!(
        !scope_verified(&path, &scope, 96).unwrap(),
        "scan completion is not verified coverage"
    );
    // G2 was outside the frozen first cycle. The next finite cycle revisits
    // G1 once before reaching G2; this is bounded, not live-upper extension.
    for through in [32, 64, 96] {
        let current = next(&path, &scope).unwrap().unwrap();
        assert_eq!(current.id, g1.id);
        assert!(finish_page(&path, &current, through).unwrap());
    }
    let g2 = next(&path, &scope).unwrap().unwrap();
    assert_ne!(g2.id, g1.id);
    assert_eq!((g2.first, g2.last), (97, 160));
    let saved = read_gap(&open_initialized(&path).unwrap(), g1.id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.cursor, 96);
    assert!(!saved.contains_verified(33));
    assert!(saved.contains_verified(96));
}
#[test]
fn cold_owner_and_same_numeric_generation_do_not_erase_unsealed_scope() {
    let (_temp, path, scope) = setup();
    discover(&path, &scope, 2).unwrap();
    prove(&path, &scope, 1, 2);
    assert!(scope_verified(&path, &scope, 2).unwrap());
    let new = Scope {
        owner_id: "new-owner".into(),
        generation: 7,
    };
    activate(&path, &new).unwrap();
    assert!(
        activate(&path, &scope).is_err(),
        "a retired observer must not reactivate over the new owner"
    );
    assert!(!scope_verified(&path, &new, 0).unwrap());
    assert!(!certify(&path, &scope, 1, &[Effect::NoRequiredStore]).unwrap());
    let db = Connection::open(&path).unwrap();
    let old:i64=db.query_row("SELECT COUNT(*) FROM cdr_observation_gaps WHERE owner_id='original' AND state='Unresolved'",[],|r|r.get(0)).unwrap();
    assert_eq!(old, 1);
}
#[test]
fn missing_or_conflicting_terminal_effect_is_not_a_successful_noop() {
    let (_temp, path, scope) = setup();
    discover(&path, &scope, 3).unwrap();
    let terminal = |payload: &str| Effect::Terminal {
        thread: "A".into(),
        turn: "T".into(),
        payload: payload.into(),
    };
    assert!(!certify(&path, &scope, 1, &[terminal("one")]).unwrap());
    let db = open_initialized(&path).unwrap();
    db.execute("INSERT INTO codex_observed_completions(thread_id,turn_id,generation,payload,resident_owner) VALUES('A','T',7,'one','original')",[]).unwrap();
    assert!(certify(&path, &scope, 1, &[terminal("one")]).unwrap());
    assert!(!certify(&path, &scope, 2, &[terminal("conflict")]).unwrap());
    assert!(!certify(&path, &scope, 3, &[Effect::Unconfirmed]).unwrap());
    assert!(!scope_verified(&path, &scope, 3).unwrap());
}
#[test]
fn aborted_proof_transaction_preserves_intent_and_cold_gap() {
    let (_temp, path, scope) = setup();
    discover(&path, &scope, 1).unwrap();
    let db = open_initialized(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_proof BEFORE UPDATE ON cdr_observation_gaps BEGIN SELECT RAISE(ABORT,'proof write failure'); END;").unwrap();
    assert!(certify(&path, &scope, 1, &[Effect::NoRequiredStore]).is_err());
    drop(db);
    assert!(!scope_verified(&path, &scope, 1).unwrap());
    let gap = next(&path, &scope).unwrap().unwrap();
    assert!(gap.verified.is_empty());
    assert_eq!(gap.cursor, 0);
}
