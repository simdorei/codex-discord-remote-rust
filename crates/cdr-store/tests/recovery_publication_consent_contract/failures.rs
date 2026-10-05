use super::*;

#[test]
fn altered_decision_revision_or_time_is_not_reported_as_committed() {
    for (revision, time) in [
        ("NEW.revision+1", "NEW.recorded_at_bits"),
        ("NEW.revision", "'0'"),
    ] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        f.db().execute_batch(&format!(
            "CREATE TRIGGER forged_receipt BEFORE INSERT ON cdr_recovery_publication_decisions
             BEGIN INSERT INTO cdr_recovery_publication_decisions
             (proposal_id,revision,ingress_id,interaction_id,decision,recorded_at_bits)
             VALUES(NEW.proposal_id,{revision},NEW.ingress_id,NEW.interaction_id,NEW.decision,{time});
             SELECT RAISE(IGNORE); END;"
        )).unwrap();
        assert!(
            f.record(&proposal, &click, 12.0).is_err(),
            "{revision} / {time}"
        );
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
        f.assert_no_execution();
    }
}

#[test]
fn custody_changed_during_insert_is_not_misread_as_a_prior_replay() {
    let f = Fixture::new();
    let proposal = f.delivered(PROPOSAL, 60);
    let click = f.click(&proposal, 70, 60, "ApproveExact");
    f.db()
        .execute_batch(
            "CREATE TRIGGER change_custody AFTER INSERT ON cdr_recovery_publication_decisions
         BEGIN UPDATE discord_ingress_journal SET state='held',phase='unknown'
         WHERE ingress_id=NEW.ingress_id; END;",
        )
        .unwrap();
    assert!(f.record(&proposal, &click, 12.0).is_err());
    assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
    let state: String = f
        .db()
        .query_row(
            "SELECT state FROM discord_ingress_journal WHERE ingress_id=?",
            [&click],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "executing");
    f.assert_no_execution();
}

#[test]
fn capability_change_before_or_during_consent_is_not_ignored() {
    for during in [false, true] {
        let f = Fixture::new();
        let proposal = f.delivered(PROPOSAL, 60);
        let click = f.click(&proposal, 70, 60, "ApproveExact");
        if during {
            f.db().execute_batch(
                "CREATE TRIGGER change_capability AFTER INSERT ON cdr_recovery_publication_decisions
                 BEGIN UPDATE cdr_runtime_capability_requirements SET format_version=2
                 WHERE component='recovery_publication_consent'; END;"
            ).unwrap();
        } else {
            f.db()
                .execute(
                    "UPDATE cdr_runtime_capability_requirements SET format_version=2
                 WHERE component='recovery_publication_consent'",
                    [],
                )
                .unwrap();
        }
        assert!(f.record(&proposal, &click, 12.0).is_err(), "{during}");
        assert_eq!(f.count("cdr_recovery_publication_decisions"), 0);
        f.assert_no_execution();
    }
}

#[test]
fn concurrent_distinct_clicks_record_exactly_one_intent() {
    let f = Fixture::new();
    let proposal = f.delivered(PROPOSAL, 60);
    let clicks = [
        f.click(&proposal, 70, 60, "ApproveExact"),
        f.click(&proposal, 71, 60, "ApproveExact"),
    ];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = clicks
        .into_iter()
        .map(|ingress| {
            let barrier = barrier.clone();
            let path = f.path.clone();
            std::thread::spawn(move || {
                barrier.wait();
                p::record_consent(
                    &path,
                    &p::ConsentInput {
                        proposal_id: PROPOSAL,
                        revision: 1,
                        ingress_id: &ingress,
                        now: 12.0,
                    },
                )
                .unwrap()
            })
        })
        .collect();
    let receipts: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_ne!(receipts[0].already_recorded, receipts[1].already_recorded);
    assert_eq!(
        receipts[0].original_ingress_id,
        receipts[1].original_ingress_id
    );
    assert_eq!(f.count("cdr_recovery_publication_decisions"), 1);
    f.assert_no_execution();
}

#[test]
fn lost_required_capability_rolls_back_initial_schema_installation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE cdr_runtime_capability_requirements(
            component TEXT PRIMARY KEY NOT NULL,format_version INTEGER NOT NULL);
         CREATE TRIGGER ignore_consent_requirement
         BEFORE INSERT ON cdr_runtime_capability_requirements
         WHEN NEW.component='recovery_publication_consent'
         BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    assert!(open_initialized(&path).is_err());
    let db = Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='cdr_recovery_publication_proposals'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
