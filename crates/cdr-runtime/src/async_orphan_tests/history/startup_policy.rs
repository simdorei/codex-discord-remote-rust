use super::*;

const INCIDENT: &str = "01a06156-56cd-70b0-af02-2de7445ba4c7";

#[tokio::test]
async fn incident_actual_writer_is_held_before_policy_is_installed() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    let result = f
        .server
        .execute(cdr_app_server::requests::resume_thread(INCIDENT), Some(1))
        .await;
    assert_eq!(
        f.calls()
            .iter()
            .filter(|v| v["method"] == "thread/resume")
            .count(),
        0,
        "unarmed incident reached the actual native writer"
    );
    assert!(result.is_err());
    let other = f
        .server
        .execute(
            cdr_app_server::requests::read_thread("unrelated", false),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(other["thread"]["id"], "unrelated");
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn incident_and_other_target_reads_remain_available() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    for thread in [INCIDENT, "unrelated"] {
        let result = f
            .server
            .execute(
                cdr_app_server::requests::read_thread(thread, false),
                Some(1),
            )
            .await
            .unwrap();
        assert_eq!(result["thread"]["id"], thread);
    }
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn owning_runtime_arms_policy_without_any_rpc_or_recovery_permission() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    let gate = crate::restart_readiness::drain::AdmissionGate::new();
    let coordinator =
        QueueCoordinator::new_with_admission_gate(f.db.clone(), f.backend.clone(), gate);
    for _ in 0..2 {
        coordinator
            .install_reviewed_recovery_policy()
            .await
            .unwrap();
    }
    let db = cdr_store::schema::open_initialized(&f.db).unwrap();
    assert!(cdr_store::async_resolution::reviewed_policy_installed_in(&db).unwrap());
    coordinator.recover_target(INCIDENT).await.unwrap();
    assert_eq!(
        f.calls().len(),
        1,
        "policy registration or held recovery emitted RPC"
    );
    assert!(cdr_store::async_resolution::admission_held(&f.db, INCIDENT).unwrap());
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn failed_registration_does_not_close_runtime_controls_or_other_target_reads() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let before = queue::list(&f.db).unwrap();
    cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_policy BEFORE INSERT ON cdr_async_recovery_policies
         BEGIN SELECT RAISE(ABORT,'injected registration failure'); END;",
        )
        .unwrap();
    let gate = crate::restart_readiness::drain::AdmissionGate::new();
    let coordinator =
        QueueCoordinator::new_with_admission_gate(f.db.clone(), f.backend.clone(), gate.clone());
    assert!(
        coordinator
            .install_reviewed_recovery_policy()
            .await
            .is_err()
    );
    let permit = gate.try_enter_control().unwrap();
    drop(permit);
    assert!(cdr_store::async_resolution::admission_held(&f.db, INCIDENT).unwrap());
    let other = f
        .server
        .execute(
            cdr_app_server::requests::read_thread("unrelated", false),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(other["thread"]["id"], "unrelated");
    f.assert_no_execution(&before);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn startup_policy_uses_original_target_lock_and_rechecks_closed_controls() {
    use crate::restart_readiness::drain::{AdmissionGate, DrainFenceKey};
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    let gate = AdmissionGate::new();
    let coordinator =
        QueueCoordinator::new_with_admission_gate(f.db.clone(), f.backend.clone(), gate.clone());
    let lock = coordinator.target_lock(INCIDENT).unwrap();
    let guard = lock.lock().await;
    let operation = coordinator.install_reviewed_recovery_policy();
    tokio::pin!(operation);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut operation)
            .await
            .is_err(),
        "registration bypassed the coordinator's original target lock"
    );
    let db = cdr_store::schema::open_initialized(&f.db).unwrap();
    assert!(!cdr_store::async_resolution::reviewed_policy_installed_in(&db).unwrap());
    let key = DrainFenceKey::new("policy-test", "1|2", "maintenance").unwrap();
    gate.seal(&key).unwrap();
    gate.close_controls(&key).unwrap();
    drop(guard);
    assert!(operation.await.is_err());
    assert!(!cdr_store::async_resolution::reviewed_policy_installed_in(&db).unwrap());
    assert!(gate.is_drained_for(&key));
    assert_eq!(f.calls().len(), 1);
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn compatibility_coordinator_without_control_gate_cannot_arm_policy() {
    let f = HistoryFixture::new(&script("completed", Some(user_input(0)))).await;
    assert!(f.queue().install_reviewed_recovery_policy().await.is_err());
    let db = cdr_store::schema::open_initialized(&f.db).unwrap();
    assert!(!cdr_store::async_resolution::reviewed_policy_installed_in(&db).unwrap());
    assert!(cdr_store::async_resolution::admission_held(&f.db, INCIDENT).unwrap());
    assert_eq!(f.calls().len(), 1);
    f.server.close().await.unwrap();
}
