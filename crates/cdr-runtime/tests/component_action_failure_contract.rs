use cdr_app_server::AppServerError;
use cdr_runtime::action_executor::ActionError;
use cdr_runtime::component_worker::{
    BusyComponentError, ClaimFailureDisposition, action_claim_failure, app_server_claim_failure,
    busy_action_claim_failure, retain_or_release_busy_claim, retain_or_release_component_claim,
};
use cdr_runtime::queue_runner::{BackendFailure, QueueRunnerError};
use cdr_store::claims::{NewBusyChoice, claim_busy_choice, claim_component, create_busy_choice};

#[test]
fn ambiguous_app_server_acceptance_keeps_the_claim() {
    let failures = [
        AppServerError::Io(std::io::Error::other("write outcome unknown")),
        AppServerError::Json(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
        AppServerError::Timeout {
            method: "server/respond".into(),
            timeout_ms: 50,
        },
        AppServerError::ResponseChannelClosed {
            method: "server/respond".into(),
        },
        AppServerError::TransportClosed {
            method: "server/respond".into(),
            reason: "app-server stdout closed".into(),
        },
        AppServerError::Closed,
    ];

    for error in failures {
        assert_eq!(
            app_server_claim_failure(&error),
            ClaimFailureDisposition::RetainIndeterminate
        );
    }
}

#[test]
fn proven_rejection_or_pre_dispatch_failure_can_release_the_claim() {
    let failures = [
        AppServerError::Remote {
            method: "server/respond".into(),
            code: -32_000,
            message: "rejected".into(),
            data: None,
        },
        AppServerError::GenerationMismatch {
            expected: 1,
            actual: 2,
        },
        AppServerError::GenerationQuarantined { generation: 1 },
    ];

    for error in failures {
        assert_eq!(
            app_server_claim_failure(&error),
            ClaimFailureDisposition::Release
        );
    }
}

#[test]
fn invalid_reply_after_dispatch_is_treated_as_indeterminate() {
    assert_eq!(
        app_server_claim_failure(&AppServerError::InvalidReply {
            message: "reply could not be validated".into(),
        }),
        ClaimFailureDisposition::RetainIndeterminate
    );
}

#[test]
fn occurrence_response_state_errors_preserve_their_write_certainty() {
    let id = cdr_app_server::RequestId::Integer(7);
    assert_eq!(
        app_server_claim_failure(&AppServerError::StaleServerRequest { id: id.clone() }),
        ClaimFailureDisposition::Release
    );
    for error in [
        AppServerError::ServerRequestResponseInFlight { id: id.clone() },
        AppServerError::ServerRequestResponseIndeterminate { id },
    ] {
        assert_eq!(
            app_server_claim_failure(&error),
            ClaimFailureDisposition::RetainIndeterminate
        );
    }
}

#[test]
fn queue_backend_ambiguity_is_preserved_for_busy_actions() {
    let ambiguous = ActionError::Queue(QueueRunnerError::Backend(BackendFailure::ambiguous(
        "turn start may have reached Codex",
    )));
    let definite = ActionError::Queue(QueueRunnerError::Backend(BackendFailure::definite(
        "request rejected before start",
    )));

    assert_eq!(
        action_claim_failure(&ambiguous),
        ClaimFailureDisposition::RetainIndeterminate
    );
    assert_eq!(
        action_claim_failure(&definite),
        ClaimFailureDisposition::Release
    );
    assert_eq!(
        busy_action_claim_failure(&BusyComponentError::Action(ambiguous)),
        ClaimFailureDisposition::RetainIndeterminate
    );
}

#[test]
fn production_release_helpers_mutate_standard_and_busy_claims_by_certainty() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    let ambiguous = AppServerError::Timeout {
        method: "server/respond".into(),
        timeout_ms: 50,
    };
    let definite = AppServerError::Remote {
        method: "server/respond".into(),
        code: -32_000,
        message: "rejected".into(),
        data: None,
    };

    assert!(claim_component(&database, "standard-ambiguous", 1.0, 100.0).unwrap());
    assert_eq!(
        retain_or_release_component_claim(&database, "standard-ambiguous", &ambiguous).unwrap(),
        ClaimFailureDisposition::RetainIndeterminate
    );
    assert!(!claim_component(&database, "standard-ambiguous", 2.0, 100.0).unwrap());

    assert!(claim_component(&database, "standard-definite", 1.0, 100.0).unwrap());
    assert_eq!(
        retain_or_release_component_claim(&database, "standard-definite", &definite).unwrap(),
        ClaimFailureDisposition::Release
    );
    assert!(claim_component(&database, "standard-definite", 2.0, 100.0).unwrap());

    let ambiguous_choice = busy_choice(&database, 10.0);
    assert!(claim_busy_choice(&database, &ambiguous_choice, 11.0).unwrap());
    let ambiguous_busy = BusyComponentError::AppServer(ambiguous);
    assert_eq!(
        retain_or_release_busy_claim(&database, &ambiguous_choice, &ambiguous_busy).unwrap(),
        ClaimFailureDisposition::RetainIndeterminate
    );
    assert!(!claim_busy_choice(&database, &ambiguous_choice, 12.0).unwrap());

    let definite_choice = busy_choice(&database, 20.0);
    assert!(claim_busy_choice(&database, &definite_choice, 21.0).unwrap());
    let definite_busy = BusyComponentError::AppServer(definite);
    assert_eq!(
        retain_or_release_busy_claim(&database, &definite_choice, &definite_busy).unwrap(),
        ClaimFailureDisposition::Release
    );
    assert!(claim_busy_choice(&database, &definite_choice, 22.0).unwrap());
}

#[test]
fn transport_closed_retain_helper_keeps_the_database_claim() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    let error = AppServerError::TransportClosed {
        method: "server/respond".into(),
        reason: "app-server stdout closed".into(),
    };

    assert!(claim_component(&database, "transport-closed", 1.0, 100.0).unwrap());
    assert_eq!(
        retain_or_release_component_claim(&database, "transport-closed", &error).unwrap(),
        ClaimFailureDisposition::RetainIndeterminate
    );
    assert!(!claim_component(&database, "transport-closed", 2.0, 100.0).unwrap());
}

fn busy_choice(database: &std::path::Path, now: f64) -> String {
    create_busy_choice(
        database,
        NewBusyChoice {
            owner_user_id: 7,
            channel_id: 8,
            target_thread_id: Some("thread-a"),
            prompt: "next",
            allow_steer: true,
            now,
            time_to_live: 100.0,
        },
    )
    .unwrap()
}
