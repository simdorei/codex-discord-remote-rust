use cdr_runtime::component_worker::{BusyComponentError, ComponentWorkerError, ConfirmationError};
use cdr_runtime::interaction_worker::{
    InteractionWorkerError,
    error_disposition::{InteractionErrorDisposition, interaction_error_disposition},
};

#[test]
fn approval_or_input_duplicate_claim_is_benign() {
    let error = InteractionWorkerError::Component(ComponentWorkerError::AlreadyHandled);
    assert_eq!(
        interaction_error_disposition(&error),
        InteractionErrorDisposition::IgnoreDuplicate
    );
}

#[test]
fn busy_duplicate_claim_is_benign() {
    let error = InteractionWorkerError::Component(ComponentWorkerError::Busy(
        BusyComponentError::AlreadyHandled,
    ));
    assert_eq!(
        interaction_error_disposition(&error),
        InteractionErrorDisposition::IgnoreDuplicate
    );
}

#[test]
fn genuine_component_failure_remains_reportable() {
    for failure in [
        ComponentWorkerError::NoPendingRequest,
        ComponentWorkerError::LegacyComponentExpired,
    ] {
        let error = InteractionWorkerError::Component(failure);
        assert_eq!(
            interaction_error_disposition(&error),
            InteractionErrorDisposition::Report
        );
    }
}

#[test]
fn post_action_confirmation_failure_is_logged_without_a_misleading_discord_error() {
    let standard = InteractionWorkerError::Component(ComponentWorkerError::Confirmation(
        ConfirmationError::Delivery("Discord timeout".into()),
    ));
    let busy = InteractionWorkerError::Component(ComponentWorkerError::Busy(
        BusyComponentError::Confirmation(ConfirmationError::Delivery("Discord timeout".into())),
    ));

    assert_eq!(
        interaction_error_disposition(&standard),
        InteractionErrorDisposition::LogOnly
    );
    assert_eq!(
        interaction_error_disposition(&busy),
        InteractionErrorDisposition::LogOnly
    );
}

#[test]
fn marker_write_failure_and_raw_claim_without_success_are_log_only() {
    let marker_write = InteractionWorkerError::Component(ComponentWorkerError::Confirmation(
        ConfirmationError::Recovery("SQLite unavailable".into()),
    ));
    let clear_failure = InteractionWorkerError::Component(ComponentWorkerError::Confirmation(
        ConfirmationError::Clear("Discord timeout".into()),
    ));
    let standard_unconfirmed =
        InteractionWorkerError::Component(ComponentWorkerError::ActionUnconfirmed);
    let busy_unconfirmed = InteractionWorkerError::Component(ComponentWorkerError::Busy(
        BusyComponentError::ActionUnconfirmed,
    ));
    let ambiguous_action = InteractionWorkerError::Component(
        ComponentWorkerError::ActionOutcomeIndeterminate("write outcome unknown".into()),
    );

    for error in [
        marker_write,
        clear_failure,
        standard_unconfirmed,
        busy_unconfirmed,
        ambiguous_action,
    ] {
        assert_eq!(
            interaction_error_disposition(&error),
            InteractionErrorDisposition::LogOnly
        );
    }
}
