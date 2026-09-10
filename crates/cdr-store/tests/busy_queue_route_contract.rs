use cdr_store::{claims, mapping, prompt_intake};

#[test]
fn changed_or_disappeared_mapping_cannot_retarget_or_claim_original_choice() {
    for missing in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        mapping::upsert_thread(&db, "A", "p", "a", 10, 42, 1.0).unwrap();
        let id = claims::create_busy_choice(
            &db,
            claims::NewBusyChoice {
                owner_user_id: 3,
                channel_id: 42,
                target_thread_id: Some("A"),
                prompt: "original prompt",
                allow_steer: false,
                now: 1.0,
                time_to_live: 100.0,
            },
        )
        .unwrap();
        let choice = claims::get_busy_choice(&db, &id, 2.0).unwrap().unwrap();
        mapping::upsert_thread(&db, "A", "p", "a", 10, 43, 2.0).unwrap();
        if !missing {
            mapping::upsert_thread(&db, "B", "p", "b", 10, 42, 2.0).unwrap();
        }
        // Try both the old target and the caller's freshly resolved target.
        for target in ["A", "B"] {
            assert!(
                prompt_intake::admit_busy_queue(&db, &choice, target, !missing, "receipt", 3.0)
                    .is_err(),
                "changed route was accepted"
            );
            assert_eq!(
                claims::get_busy_choice(&db, &id, 3.0).unwrap(),
                Some(choice.clone())
            );
            assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
            assert_eq!(claims::component_claim_counts(&db, 3.0).unwrap(), (0, 0));
        }
    }
}

#[test]
fn unchanged_mapping_accepts_once_and_legacy_route_is_explicitly_expired() {
    for legacy in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        mapping::upsert_thread(&db, "A", "p", "a", 10, 42, 1.0).unwrap();
        let id = claims::create_busy_choice_on_route(&db, new_choice(), true).unwrap();
        let choice = claims::get_busy_choice(&db, &id, 2.0).unwrap().unwrap();
        if legacy {
            cdr_store::schema::open_initialized(&db)
                .unwrap()
                .execute(
                    "UPDATE busy_choices SET require_current_mirror=NULL WHERE choice_id=?",
                    [&id],
                )
                .unwrap();
        }
        let result = prompt_intake::admit_busy_queue(&db, &choice, "A", true, "receipt", 3.0);
        if legacy {
            assert!(matches!(
                result,
                Err(cdr_store::StoreError::BusyChoiceUnavailable(_))
            ));
            assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
            assert_eq!(claims::component_claim_counts(&db, 3.0).unwrap(), (0, 0));
        } else {
            assert_eq!(result.unwrap().intake.unwrap().target_thread_id, "A");
            assert!(
                prompt_intake::admit_busy_queue(&db, &choice, "A", true, "receipt", 4.0)
                    .unwrap()
                    .intake
                    .is_none()
            );
            assert_eq!(prompt_intake::list_prompt_intakes(&db).unwrap().len(), 1);
        }
    }
}

#[test]
fn changed_mapping_at_busy_creation_never_displays_a_rebound_choice() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    assert!(claims::create_busy_choice_on_route(&db, new_choice(), true).is_err());
    mapping::upsert_thread(&db, "B", "p", "b", 10, 42, 1.0).unwrap();
    assert!(claims::create_busy_choice_on_route(&db, new_choice(), true).is_err());
    assert!(claims::create_busy_choice_on_route(&db, new_choice(), false).is_err());
    assert_eq!(claims::busy_choice_counts(&db, 2.0).unwrap(), (0, 0));
}

#[test]
fn mapping_writer_commits_before_waiting_admission_without_cross_thread_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    mapping::upsert_thread(&db, "A", "p", "a", 10, 42, 1.0).unwrap();
    let id = claims::create_busy_choice_on_route(&db, new_choice(), true).unwrap();
    let choice = claims::get_busy_choice(&db, &id, 2.0).unwrap().unwrap();
    let mut connection = cdr_store::schema::open_initialized(&db).unwrap();
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "UPDATE mirror_threads SET discord_thread_id=43 WHERE codex_thread_id='A'",
        [],
    )
    .unwrap();
    let (started, recv) = std::sync::mpsc::channel();
    let waiting = std::thread::spawn({
        let (db, choice) = (db.clone(), choice.clone());
        move || {
            started.send(()).unwrap();
            prompt_intake::admit_busy_queue(&db, &choice, "A", true, "receipt", 3.0)
        }
    });
    recv.recv().unwrap();
    tx.commit().unwrap();
    assert!(waiting.join().unwrap().is_err());
    assert_eq!(
        claims::get_busy_choice(&db, &id, 3.0).unwrap(),
        Some(choice)
    );
    assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
    assert_eq!(claims::component_claim_counts(&db, 3.0).unwrap(), (0, 0));
}

fn new_choice() -> claims::NewBusyChoice<'static> {
    claims::NewBusyChoice {
        owner_user_id: 3,
        channel_id: 42,
        target_thread_id: Some("A"),
        prompt: "original prompt",
        allow_steer: false,
        now: 1.0,
        time_to_live: 100.0,
    }
}

#[test]
fn shared_old_store_migration_preserves_choice_but_does_not_guess_its_route() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("old.sqlite");
    let old = rusqlite::Connection::open(&db).unwrap();
    old.execute_batch(include_str!(
        "../../../fixtures/parity/discord_mirror_schema_v2.sql"
    ))
    .unwrap();
    old.execute(
        "INSERT INTO busy_choices VALUES ('legacy',3,42,'A','original prompt',0,1,101,NULL)",
        [],
    )
    .unwrap();
    drop(old);
    let current = cdr_store::schema::open_initialized(&db).unwrap();
    let mode: Option<bool> = current
        .query_row(
            "SELECT require_current_mirror FROM busy_choices WHERE choice_id='legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mode, None);
    let choice = claims::get_busy_choice(&db, "legacy", 2.0)
        .unwrap()
        .unwrap();
    assert_eq!(choice.prompt, "original prompt");
    assert_eq!(choice.target_thread_id.as_deref(), Some("A"));
    assert!(prompt_intake::admit_busy_queue(&db, &choice, "A", false, "receipt", 3.0).is_err());
    assert_eq!(
        claims::get_busy_choice(&db, "legacy", 3.0).unwrap(),
        Some(choice)
    );
    let backups: Vec<_> = std::fs::read_dir(temp.path().join(".codex-discord-backups"))
        .unwrap()
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = rusqlite::Connection::open(backups[0].as_ref().unwrap().path()).unwrap();
    assert_eq!(
        backup
            .query_row(
                "SELECT prompt FROM busy_choices WHERE choice_id='legacy'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "original prompt"
    );
}
