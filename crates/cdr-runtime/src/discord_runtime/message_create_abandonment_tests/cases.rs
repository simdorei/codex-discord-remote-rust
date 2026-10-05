use super::*;

#[test]
fn dedicated_discard_grammar_and_component_do_not_alias_native_approval() {
    use crate::{
        message_plan::{IncomingMessage, plan_message},
        prefix_plan::plan_prefix,
    };
    let content = format!("!discard-request {JOB}");
    let incoming = IncomingMessage {
        content: &content,
        message_content_enabled: true,
        channel_allowed: true,
        user_allowed: true,
        author_is_bot: false,
        author_is_self: false,
        author_mentions_bridge: false,
        has_attachments: false,
        mirrored_target: true,
        mentioned_user_ids: std::collections::BTreeSet::default(),
        required_plain_ask_user_ids: std::collections::BTreeSet::default(),
    };
    assert_eq!(
        serde_json::to_value(plan_message(&incoming).unwrap()).unwrap(),
        json!({"Execute":{"DiscardRequest":{"job_id":JOB}}})
    );
    let mut bot = incoming.clone();
    bot.author_is_bot = true;
    bot.author_mentions_bridge = true;
    assert!(!matches!(
        plan_message(&bot).unwrap(),
        crate::message_plan::MessagePlan::Execute(_)
    ));
    for value in [
        "discard-request",
        "discard-request bad",
        "discard-request thread-b",
        "discard-request b3d5a1a35c3e4764967b0cef767efde9",
        "discard-request B3D5A1A3-5C3E-4764-967B-0CEF767EFDE9",
    ] {
        assert!(plan_prefix(value).is_err());
    }
    assert!(plan_prefix(&format!("discard-request {JOB} extra")).is_err());
    for choice in ["a", "h"] {
        let id =
            parse_component_id(&format!("codex_discard:v1:{}:1:{choice}", "c".repeat(32))).unwrap();
        assert!(build_component_response(&id, &[], 1).is_err());
        assert!(standard_confirmation_plan(&id, "native").is_err());
        assert!(cdr_discord::components::persistent_component_claim_key(60, &id).is_none());
    }
    for revision in ["0", "-1", "01", "+1"] {
        assert!(
            parse_component_id(&format!("codex_discard:v1:{}:{revision}:a", "c".repeat(32)))
                .is_none()
        );
    }
}

#[tokio::test]
async fn real_gateway_and_interaction_worker_dispose_only_one_request_without_rpc() {
    let http = http::start(None).await;
    let f = Fixture::new(&http.address).await;
    f.propose(801).await;
    f.held(0);
    let p = f.proposal();
    assert!(!p.review_text.contains("private original input"));
    let work = f.click(501, "a").await;
    assert!(work.admission_permit.is_some());
    let row = ingress::get(f.db(), &work.custody_ingress_id)
        .unwrap()
        .unwrap();
    assert_eq!(row.target_thread_id.as_deref(), Some(TARGET));
    let db = f.db().to_path_buf();
    let (sender, receiver) = mpsc::channel(1);
    sender.send(work).await.unwrap();
    drop(sender);
    let runtime = f.runtime;
    tokio::time::timeout(
        Duration::from_secs(10),
        crate::interaction_worker::run_interaction_worker(
            receiver,
            Arc::new(runtime.executor),
            runtime.server.clone(),
            runtime.http.clone(),
        ),
    )
    .await
    .unwrap();
    let receipt = store::decision_status(&db, &p.id, p.revision)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.decision, store::Decision::AbandonOnly);
    assert_eq!(receipt.ingress_id, "interaction:501");
    let rows = queue::list(&db).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].job_id, SIBLING);
    assert!(async_resolution::admission_held(&db, TARGET).unwrap());
    assert_eq!(
        ingress::get(&db, "interaction:501").unwrap().unwrap().state,
        "completed"
    );
    assert!(matches!(
        queue::try_begin_attempt(&db, SIBLING, &[], 1),
        Err(cdr_store::StoreError::AsyncResolutionHeld { .. })
    ));
    runtime.server.close().await.unwrap();
    assert_no_rpc(&f.temp);
    let traffic = http.finish().await;
    assert_eq!(
        traffic
            .iter()
            .filter(|(r, _)| r.starts_with("POST "))
            .count(),
        2
    );
    assert_eq!(
        traffic
            .iter()
            .filter(|(r, _)| r.starts_with("PATCH "))
            .count(),
        1
    );
    assert!(
        traffic[1].1["content"]
            .as_str()
            .unwrap()
            .contains("remains held")
    );
}

#[tokio::test]
async fn actual_owner_and_normal_admission_are_required_before_any_proposal() {
    let http = http::start(None).await;
    let f = Fixture::new(&http.address).await;
    let mut other = raw_message(801, &format!("!discard-request {JOB}"));
    other.author.id = Id::new(4);
    assert!(f.message(other).await.is_err());
    assert_eq!(f.count("cdr_recovery_abandonment_proposals"), 0);
    let context = f.runtime.context(f.temp.path());
    let admitted = f.runtime.admit_id(&format!("!discard-request {JOB}"), 802);
    assert!(
        process_admitted_gateway_message(admitted, &context)
            .await
            .is_err()
    );
    let key = DrainFenceKey::new("fixture", "1|1", "nonce").unwrap();
    f.gate.seal(&key).unwrap();
    assert!(
        f.message(raw_message(803, &format!("!discard-request {JOB}")))
            .await
            .is_err()
    );
    assert_eq!(f.count("cdr_recovery_abandonment_proposals"), 0);
    f.held(0);
    f.finish().await;
    assert!(http.finish().await.is_empty());
}

#[tokio::test]
async fn exact_headers_and_latest_proposal_are_checked_before_ack() {
    let http = http::start(None).await;
    let f = Fixture::new(&http.address).await;
    f.propose(801).await;
    let p = f.proposal();
    let custom = format!("codex_discard:v1:{}:{}:a", p.id, p.revision);
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
    assert_eq!(f.transport.acknowledgements.load(Ordering::SeqCst), 0);
    f.sql("UPDATE mirror_threads SET discord_thread_id=43");
    assert!(f.dispatch(raw_click(502, &custom)).await.is_err());
    assert_eq!(f.transport.acknowledgements.load(Ordering::SeqCst), 0);
    f.held(0);
    f.finish().await;
    assert_eq!(http.finish().await.len(), 1);
}

#[tokio::test]
async fn worker_rechecks_actual_envelope_and_cancellation_keeps_original_rows() {
    let http = http::start(None).await;
    let f = Fixture::new(&http.address).await;
    f.propose(801).await;
    let work = f.click(501, "a").await;
    f.begin(&work);
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
            _ => bad.custody_database = f.temp.path().join("different.sqlite"),
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
    let lock = f.runtime.executor.control_lock(TARGET).await.unwrap();
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
    drop(pending);
    f.held(0);
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
    f.sql("UPDATE mirror_threads SET discord_thread_id=43");
    drop(lock);
    assert!(pending.await.is_err());
    f.held(0);
    drop(work);
    f.finish().await;
    assert_eq!(http.finish().await.len(), 1);
}

#[tokio::test]
async fn unknown_proposal_response_and_binding_failure_never_repost() {
    for failure in 0..3 {
        let http = http::start((failure == 0).then_some(1)).await;
        let f = Fixture::new(&http.address).await;
        match failure {
            1 => f.sql(
                "CREATE TRIGGER fail_binding BEFORE INSERT ON cdr_recovery_abandonment_deliveries
                BEGIN SELECT RAISE(ABORT,'injected binding failure'); END;",
            ),
            2 => f.sql(
                "CREATE TRIGGER fail_receipt BEFORE UPDATE OF message_id ON codex_delivery_receipts
                BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;",
            ),
            _ => {}
        }
        assert!(
            f.message(raw_message(801, &format!("!discard-request {JOB}")))
                .await
                .is_err()
        );
        f.message(raw_message(801, &format!("!discard-request {JOB}")))
            .await
            .unwrap();
        assert_eq!(f.count("cdr_recovery_abandonment_proposals"), 1);
        assert_eq!(f.count("cdr_recovery_abandonment_deliveries"), 0);
        f.held(0);
        f.finish().await;
        assert_eq!(http.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn unknown_decision_notification_cannot_reapply_or_borrow_another_click() {
    let http = http::start(Some(2)).await;
    let f = Fixture::new(&http.address).await;
    f.propose(801).await;
    let work = f.click(501, "h").await;
    f.begin(&work);
    let p = f.proposal();
    for _ in 0..2 {
        let confirmation = handle_component_work(
            &work,
            component(&work),
            &f.runtime.executor,
            &f.runtime.server,
        )
        .await
        .unwrap();
        assert!(confirmation.deliver(f.runtime.http.clone()).await.is_err());
        f.held(1);
    }
    for choice in ["a", "h"] {
        let custom = format!("codex_discard:v1:{}:{}:{choice}", p.id, p.revision);
        assert!(f.dispatch(raw_click(502, &custom)).await.is_err());
    }
    assert_eq!(f.transport.acknowledgements.load(Ordering::SeqCst), 1);
    drop(work);
    f.finish().await;
    assert_eq!(http.finish().await.len(), 2);
}
