use super::*;

#[tokio::test]
async fn original_authenticated_headers_are_checked_before_acknowledgement_and_recording() {
    let http = http::start(false).await;
    let f = Fixture::new(&http.address).await;
    let custom = format!("codex_pub:v1:{ID}:1:a");
    for (pointer, bad) in [
        ("/application_id", "2"),
        ("/channel_id", "43"),
        ("/user/id", "4"),
        ("/message/id", "61"),
    ] {
        let mut value = raw_click(501, &custom);
        *value.pointer_mut(pointer).unwrap() = json!(bad);
        assert!(f.dispatch(value).await.is_err());
    }
    let db = f.runtime.executor.mirror_db();
    let count: i64 = cdr_store::schema::open_initialized(db)
        .unwrap()
        .query_row("SELECT count(*) FROM discord_ingress_journal", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    f.assert_held(0);
    f.finish().await;
    assert!(http.finish().await.is_empty());
}

#[tokio::test]
async fn worker_rechecks_custody_envelope_admission_and_confirmation_mode() {
    let http = http::start(false).await;
    let f = Fixture::new(&http.address).await;
    let work = f.click(501, "a").await;
    for case in 0..8 {
        let mut bad = work.clone();
        match case {
            0 => bad.application_id = Id::new(2),
            1 => bad.channel_id = Id::new(43),
            2 => bad.user_id = Id::new(4),
            3 => bad.source_message_id = None,
            4 => bad.interaction_id = Id::new(502),
            5 => bad.admission_permit = None,
            6 => bad.processing_mode = InteractionProcessingMode::ConfirmationOnly,
            _ => {
                bad.work = RoutedWork::Component(ComponentId::RecoveryPublicationDecision {
                    proposal_id: ID.into(),
                    revision: 1,
                    decision: PublicationDecision::KeepHeld,
                });
            }
        }
        assert!(
            handle_component_work(
                &bad,
                component(&work),
                &f.runtime.executor,
                &f.runtime.server
            )
            .await
            .is_err()
        );
    }
    f.assert_held(0);
    drop(work);
    f.finish().await;
    assert!(http.finish().await.is_empty());
}

#[tokio::test]
async fn target_lock_cancellation_and_evidence_change_preserve_held_pending() {
    let http = http::start(false).await;
    let f = Fixture::new(&http.address).await;
    let work = f.click(501, "a").await;
    let lock = f.runtime.executor.control_lock(TARGET).await.unwrap();
    let mut cancelled = Box::pin(handle_component_work(
        &work,
        component(&work),
        &f.runtime.executor,
        &f.runtime.server,
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut cancelled)
            .await
            .is_err()
    );
    drop(cancelled);
    f.assert_held(0);
    let mut pending = Box::pin(handle_component_work(
        &work,
        component(&work),
        &f.runtime.executor,
        &f.runtime.server,
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut pending)
            .await
            .is_err()
    );
    cdr_store::schema::open_initialized(f.runtime.executor.mirror_db()).unwrap().execute(
        "UPDATE codex_turn_queue SET prompt='evidence changed while locked' WHERE job_id='publication-pending'",
        [],
    ).unwrap();
    drop(lock);
    assert!(pending.await.is_err());
    f.assert_held(0);
    drop(work);
    f.finish().await;
    assert!(http.finish().await.is_empty());
}

#[tokio::test]
async fn preexisting_admission_drains_and_sealed_gate_rejects_new_intent() {
    let http = http::start(false).await;
    let f = Fixture::new(&http.address).await;
    let work = f.click(501, "h").await;
    let key = DrainFenceKey::new("fixture", "1|1", "nonce").unwrap();
    f.gate.seal(&key).unwrap();
    assert!(!f.gate.is_drained_for(&key));
    assert!(
        f.dispatch(raw_click(502, &format!("codex_pub:v1:{ID}:1:a")))
            .await
            .is_err()
    );
    let confirmation = handle_component_work(
        &work,
        component(&work),
        &f.runtime.executor,
        &f.runtime.server,
    )
    .await
    .unwrap();
    assert!(confirmation.plan.content.contains("remain held"));
    f.assert_held(1);
    drop(work);
    assert!(f.gate.is_drained_for(&key));
    f.finish().await;
    assert!(http.finish().await.is_empty());
}

#[tokio::test]
async fn conflicting_choice_and_changed_mapping_cannot_replace_recorded_intent() {
    let http = http::start(false).await;
    let f = Fixture::new(&http.address).await;
    let first = f.click(501, "h").await;
    handle_component_work(
        &first,
        component(&first),
        &f.runtime.executor,
        &f.runtime.server,
    )
    .await
    .unwrap();
    let conflict = f.click(502, "a").await;
    assert!(
        handle_component_work(
            &conflict,
            component(&conflict),
            &f.runtime.executor,
            &f.runtime.server
        )
        .await
        .is_err()
    );
    let duplicate = f.click(503, "h").await;
    cdr_store::schema::open_initialized(f.runtime.executor.mirror_db())
        .unwrap()
        .execute(
            "UPDATE mirror_threads SET discord_thread_id=43 WHERE codex_thread_id=?",
            [TARGET],
        )
        .unwrap();
    assert!(
        handle_component_work(
            &duplicate,
            component(&duplicate),
            &f.runtime.executor,
            &f.runtime.server
        )
        .await
        .is_err()
    );
    f.assert_held(1);
    drop((first, conflict, duplicate));
    f.finish().await;
    assert!(http.finish().await.is_empty());
}
