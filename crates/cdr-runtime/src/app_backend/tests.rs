use cdr_app_server::AppServerError;

use super::{mutation_failure, resume_failure, start_failure};
use crate::queue_runner::BackendFailureKind;

#[test]
fn turn_start_transport_closure_is_ambiguous_but_remote_rejection_is_definite() {
    let transport = start_failure(&AppServerError::TransportClosed {
        method: "turn/start".into(),
        reason: "app-server stdout closed".into(),
    });
    assert!(transport.ambiguous);

    let remote = start_failure(&AppServerError::Remote {
        method: "turn/start".into(),
        code: -32_000,
        message: "rejected".into(),
        data: None,
    });
    assert!(!remote.ambiguous);
}

#[test]
fn resume_active_writer_is_a_typed_definite_failure() {
    let failure = resume_failure(&AppServerError::Remote {
        method: "thread/resume".into(),
        code: -32_600,
        message: "thread thread-a already has an active writer".into(),
        data: None,
    });

    assert!(!failure.ambiguous);
    assert_eq!(failure.kind, BackendFailureKind::ActiveWriter);
}

#[test]
fn fork_transport_failure_is_ambiguous_but_remote_rejection_is_definite() {
    let transport = mutation_failure(&AppServerError::TransportClosed {
        method: "thread/fork".into(),
        reason: "app-server stdout closed".into(),
    });
    assert!(transport.ambiguous);

    let remote = mutation_failure(&AppServerError::Remote {
        method: "thread/fork".into(),
        code: -32_600,
        message: "rejected".into(),
        data: None,
    });
    assert!(!remote.ambiguous);
}
