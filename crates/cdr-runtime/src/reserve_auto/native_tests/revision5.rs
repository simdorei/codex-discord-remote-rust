use super::*;
use cdr_store::{archive_fence, reserve_policy};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[tokio::test]
async fn f1_clean_ordinary_unknown_quota_preserves_original_first_start() {
    let fixture = Fixture::new().await;
    fixture.configure(json!({"ordinary": null})).await;

    let result = fixture
        .queue
        .submit_identified("f1-clean-ordinary-unknown", "thread-b", 42, 3, None, "f1")
        .await;

    assert!(
        result.is_ok(),
        "a clean ordinary request must remain startable when quota is unknown: {result:?}"
    );
    assert_eq!(fixture.count("thread/settings/update"), 0);
    assert_eq!(fixture.count("turn/start"), 1);

    fixture.close().await;
}

#[tokio::test]
async fn f1_known_usage_failure_with_unknown_quota_remains_held() {
    let fixture = Fixture::new().await;
    fixture.configure(json!({"ordinary": null})).await;
    fixture.controller.note_usage_limit("thread-b").await;
    assert_eq!(
        reserve_policy::get(&fixture.db, "thread-b")
            .unwrap()
            .unwrap()
            .state,
        "unknown"
    );

    let result = fixture
        .queue
        .submit_identified("f1-known-failure", "thread-b", 42, 3, None, "f1")
        .await;
    assert!(result.is_err());
    assert_eq!(fixture.count("turn/start"), 0);
    assert_eq!(fixture.count("thread/settings/update"), 0);

    fixture.close().await;
}

#[tokio::test]
async fn r5_1_persisted_usage_failure_fence_blocks_followup_unknown_quota() {
    let fixture = Fixture::new().await;
    reserve_policy::ensure(&fixture.db, "thread-b").unwrap();
    reserve_policy::stage_usage_failure(
        &fixture.db,
        "thread-b",
        "injected unresolved typed usage failure",
    )
    .unwrap();
    fixture.configure(json!({"ordinary": null})).await;

    let result = fixture
        .queue
        .submit_identified("f1-followup-after-failure", "thread-b", 42, 3, None, "f1")
        .await;

    assert!(result.is_err());
    assert_eq!(fixture.count("turn/start"), 0);
    assert!(reserve_policy::usage_failure_unresolved(&fixture.db, "thread-b").unwrap());

    fixture.close().await;
}

#[tokio::test]
async fn f2_archive_attempted_fence_blocks_background_restore() {
    let fixture = Fixture::new().await;
    fixture
        .controller
        .prepare_turn("thread-b")
        .await
        .expect("automatic Reserve entry should succeed");
    fixture.configure(json!({"ordinary": true})).await;

    let before_resume = fixture.count("thread/resume");
    let before_update = fixture.count("thread/settings/update");
    let operation =
        archive_fence::reserve(&fixture.db, &BTreeSet::from(["thread-b".to_string()]), None)
            .expect("archive fence reservation should succeed");

    fixture
        .queue
        .recover_reserve_target(&fixture.controller, "thread-b")
        .await
        .expect("a fenced recovery should be a handled no-op");

    assert_eq!(fixture.count("thread/resume"), before_resume);
    assert_eq!(fixture.count("thread/settings/update"), before_update);
    assert_eq!(
        reserve_policy::get(&fixture.db, "thread-b")
            .unwrap()
            .unwrap()
            .state,
        "reserve"
    );
    let connection = rusqlite::Connection::open(&fixture.db).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT phase FROM codex_archive_fences WHERE target_thread_id=?1",
                ["thread-b"],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "attempted"
    );

    archive_fence::verified(&fixture.db, &operation).unwrap();
    fixture
        .queue
        .recover_reserve_target(&fixture.controller, "thread-b")
        .await
        .expect("a verified archive fence should also be a handled no-op");
    assert_eq!(fixture.count("thread/resume"), before_resume);
    assert_eq!(fixture.count("thread/settings/update"), before_update);
    let connection = rusqlite::Connection::open(&fixture.db).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT phase FROM codex_archive_fences WHERE target_thread_id=?1",
                ["thread-b"],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "verified"
    );

    fixture.close().await;
}

#[tokio::test]
async fn r7_pending_failure_in_verified_reserve_is_resolved_before_followup_start() {
    let fixture = Fixture::new().await;
    fixture
        .controller
        .prepare_turn("thread-b")
        .await
        .expect("automatic Reserve entry should succeed");
    reserve_policy::stage_usage_failure(&fixture.db, "thread-b", "pending Reserve failure")
        .unwrap();
    fixture.configure(json!({"ordinary": false})).await;
    let before_updates = fixture.count("thread/settings/update");

    fixture
        .controller
        .prepare_turn("thread-b")
        .await
        .expect("verified Reserve should resolve its pending failure before allowing work");
    assert!(!reserve_policy::usage_failure_unresolved(&fixture.db, "thread-b").unwrap());
    assert_eq!(fixture.count("thread/settings/update"), before_updates);

    let followup = fixture.submit("r7-followup").await;
    assert!(followup.turn_id.is_some());
    assert_eq!(fixture.count("turn/start"), 1);
    fixture
        .finish(followup.turn_id.as_deref().unwrap(), false)
        .await;
    fixture.close().await;
}

#[tokio::test]
async fn historical_terminal_rechecks_current_policy_without_replaying_the_original() {
    let fixture = Fixture::new().await;
    fixture.configure(json!({"ordinary":true})).await;
    let submitted = fixture.submit("r9-old-generation").await;
    let turn_id = submitted.turn_id.as_deref().unwrap();
    let response = fixture
        .rpc(
            "test/complete",
            json!({"turnId": turn_id, "status": "failed"}),
        )
        .await;
    let completion = parse_turn_completion(&response, false).unwrap();
    assert!(completion.usage_limit);
    let original_generation = cdr_store::queue::list(&fixture.db)
        .unwrap()
        .into_iter()
        .find(|job| job.job_id == "r9-old-generation")
        .unwrap()
        .app_server_generation;
    let starts_before_recovery = fixture.count("turn/start");
    let settings_updates_before_recovery = fixture.count("thread/settings/update");

    assert!(fixture.server.force_restart_if_quiescent().await.unwrap());
    let current_generation = i64::try_from(fixture.server.generation()).unwrap();
    assert_ne!(current_generation, original_generation);
    fixture.configure(json!({"ordinary": false})).await;
    let controller = ReserveAutoController::new(fixture.server.clone(), fixture.db.clone());
    let recovered = QueueCoordinator::new_with_admission_gate(
        fixture.db.clone(),
        Arc::new(AppServerTurnBackend::new(fixture.server.clone()).with_reserve_auto(controller)),
        crate::restart_readiness::drain::AdmissionGate::new(),
    );
    let report = recovered.recover().await.unwrap();
    assert!(report.unresolved > 0);
    assert_eq!(report.adopted, 0);

    recovered
        .stage_turn_completion_with_usage_limit(
            "thread-b",
            turn_id,
            "historical failed completion",
            completion.usage_limit,
        )
        .await
        .unwrap();

    assert_eq!(fixture.count("turn/start"), starts_before_recovery);
    assert_eq!(
        fixture.count("thread/settings/update"),
        settings_updates_before_recovery + 1
    );
    assert!(!reserve_policy::usage_failure_unresolved(&fixture.db, "thread-b").unwrap());
    fixture.close().await;
}

#[tokio::test]
async fn r8_existing_reserve_settings_cannot_bypass_a_pending_fence() {
    let fixture = Fixture::new().await;
    fixture
        .executor
        .execute(
            CommandAction::Settings {
                reference: Some("thread-b".into()),
                model: Some("gpt-reserve".into()),
                effort: Some("high".into()),
                speed: None,
            },
            42,
            3,
        )
        .await
        .unwrap();
    fixture
        .controller
        .set_manual_mode("thread-b", true)
        .unwrap();
    reserve_policy::stage_usage_failure(
        &fixture.db,
        "thread-b",
        "pending existing Reserve failure",
    )
    .unwrap();
    fixture.configure(json!({"ordinary": false})).await;
    let before_updates = fixture.count("thread/settings/update");

    let followup = fixture.submit("r8-existing-reserve-followup").await;
    assert!(followup.turn_id.is_some());
    assert!(!reserve_policy::usage_failure_unresolved(&fixture.db, "thread-b").unwrap());
    // R15: enabling auto on a manually selected high aligns once to default medium.
    assert_eq!(fixture.count("thread/settings/update"), before_updates + 1);
    assert_eq!(
        fixture
            .frames()
            .iter()
            .find(|row| row["event"] == "start_settings")
            .unwrap()["settings"]["effort"],
        "medium"
    );
    fixture
        .finish(followup.turn_id.as_deref().unwrap(), false)
        .await;
    fixture.close().await;
}

#[tokio::test]
async fn r8_ownerless_pending_fence_blocks_admission_after_successful_episode() {
    let fixture = Fixture::new().await;
    reserve_policy::ensure(&fixture.db, "thread-b").unwrap();
    reserve_policy::stage_usage_failure(&fixture.db, "thread-b", "legacy pending failure").unwrap();
    rusqlite::Connection::open(&fixture.db)
        .unwrap()
        .execute(
            "UPDATE codex_reserve_policy SET usage_failure_revision=NULL WHERE thread_id=?1",
            ["thread-b"],
        )
        .unwrap();
    fixture.configure(json!({"ordinary": false})).await;

    let result = fixture
        .queue
        .submit_identified("r8-ownerless-followup", "thread-b", 42, 3, None, "r8")
        .await;
    assert!(result.is_err());
    assert_eq!(fixture.count("turn/start"), 0);
    assert_eq!(fixture.count("thread/settings/update"), 1);
    assert!(reserve_policy::usage_failure_unresolved(&fixture.db, "thread-b").unwrap());
    fixture.close().await;
}

#[tokio::test]
async fn f3_automatic_transitions_stage_distinct_revision_bound_notices() {
    let fixture = Fixture::new().await;
    fixture
        .controller
        .prepare_turn("thread-b")
        .await
        .expect("automatic Reserve entry should succeed");
    let reserve_notice = reserve_policy::transition_notice::pending(&fixture.db)
        .unwrap()
        .pop()
        .expect("Reserve entry notice should be durable");
    assert_eq!(reserve_notice.channel_id, Some(42));
    assert!(reserve_notice.content.contains("자동 Reserve 전환 확인"));
    assert!(reserve_notice.content.contains("model: "));
    assert!(reserve_notice.content.contains("policy revision:"));
    assert!(
        reserve_notice
            .content
            .contains("이전 실패 요청은 자동 재실행하지 않습니다.")
    );
    let key = serde_json::to_string(&(
        reserve_notice.channel_id.unwrap(),
        reserve_policy::transition_notice::DOMAIN,
        &reserve_notice.notice_id,
        0usize,
    ))
    .unwrap();
    let hash = hex::encode(Sha256::digest(reserve_notice.content.as_bytes()));
    assert_eq!(
        cdr_store::delivery_receipt::begin(&fixture.db, &key, &hash).unwrap(),
        cdr_store::delivery_receipt::ReceiptState::New
    );
    assert_eq!(
        cdr_store::delivery_receipt::begin(&fixture.db, &key, &hash).unwrap(),
        cdr_store::delivery_receipt::ReceiptState::Unknown
    );

    fixture.configure(json!({"ordinary": true})).await;
    fixture
        .controller
        .prepare_turn("thread-b")
        .await
        .expect("automatic restore should succeed");
    let notices = reserve_policy::transition_notice::pending(&fixture.db).unwrap();
    assert_eq!(notices.len(), 2);
    assert_ne!(notices[0].notice_id, notices[1].notice_id);
    assert!(notices[1].content.contains("일반 설정 자동 복귀 확인"));

    fixture
        .controller
        .prepare_turn("thread-b")
        .await
        .expect("ordinary state should remain a no-op");
    assert_eq!(
        reserve_policy::transition_notice::pending(&fixture.db)
            .unwrap()
            .len(),
        2
    );

    fixture.close().await;
}

#[tokio::test]
async fn f3_manual_settings_result_explains_automatic_policy_takeover() {
    let fixture = Fixture::new().await;
    let result = fixture
        .executor
        .execute(
            CommandAction::Settings {
                reference: Some("thread-b".into()),
                model: Some("model-b".into()),
                effort: Some("medium".into()),
                speed: None,
            },
            42,
            3,
        )
        .await
        .unwrap();
    assert!(
        result
            .text
            .contains("자동 Reserve 전환·복귀 정책: 사용자 수동 설정으로 해제됨")
    );
    assert_eq!(
        reserve_policy::get(&fixture.db, "thread-b")
            .unwrap()
            .unwrap()
            .mode,
        "manual"
    );
    assert_eq!(fixture.count("turn/start"), 0);

    fixture.close().await;
}
