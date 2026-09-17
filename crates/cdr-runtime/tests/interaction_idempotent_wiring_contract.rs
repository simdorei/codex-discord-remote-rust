const INTERACTION_WORKER: &str = include_str!("../src/interaction_worker.rs");
const INTERACTION_ERROR_REPORT: &str = include_str!("../src/interaction_worker/error_report.rs");
const INTERACTION_DELIVERY: &str = include_str!("../src/interaction_worker/delivery.rs");
const COMPONENT_WORKER: &str = include_str!("../src/component_worker.rs");
const BUSY_COMPONENT_WORKER: &str = include_str!("../src/component_worker/busy.rs");
const COMPONENT_CONFIRMATION: &str = include_str!("../src/component_worker/confirmation.rs");

#[test]
fn interaction_create_message_paths_use_stable_idempotent_delivery() {
    assert!(!INTERACTION_WORKER.contains("api.send_message("));
    assert!(!INTERACTION_WORKER.contains("api.deliver_interaction_text("));
    assert!(INTERACTION_WORKER.contains("mod error_report;"));
    assert!(INTERACTION_WORKER.contains("use error_report::report_interaction_error;"));
    assert!(INTERACTION_WORKER.contains("report_interaction_error(&work, error,"));
    assert!(!INTERACTION_ERROR_REPORT.contains("api.send_message("));
    assert!(!INTERACTION_ERROR_REPORT.contains("api.deliver_interaction_text("));
    assert!(INTERACTION_ERROR_REPORT.contains("deliver_interaction_text_idempotent("));
    assert!(INTERACTION_ERROR_REPORT.contains("send_recorded_message_with_components("));
    assert!(INTERACTION_ERROR_REPORT.contains("&work.custody_database"));
    assert!(INTERACTION_ERROR_REPORT.contains("InteractionErrorDisposition::IgnoreDuplicate"));
    assert!(INTERACTION_DELIVERY.contains("update_initial_response("));
    assert!(INTERACTION_DELIVERY.contains("send_idempotent_message("));
    assert!(!INTERACTION_DELIVERY.contains(".send_message("));
}

#[test]
fn component_confirmations_use_stable_idempotent_delivery() {
    assert!(!COMPONENT_WORKER.contains(".send_message("));
    assert!(!BUSY_COMPONENT_WORKER.contains(".send_message("));
    assert!(!COMPONENT_CONFIRMATION.contains(".send_message("));
    assert!(COMPONENT_CONFIRMATION.contains("send_recorded_message_with_components("));
    assert!(COMPONENT_CONFIRMATION.contains("logical_key: plan.logical_key.clone()"));
    assert!(COMPONENT_CONFIRMATION.contains("send_then_clear("));
}
