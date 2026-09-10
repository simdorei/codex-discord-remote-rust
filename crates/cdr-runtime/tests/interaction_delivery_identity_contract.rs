use cdr_discord::components::{ApprovalAnswer, BusyAction, ComponentId};
use cdr_runtime::discord_dispatch::delivery_identity::{
    BUSY_CONFIRMATION_DOMAIN, COMPONENT_CONFIRMATION_DOMAIN, COMPONENT_ERROR_DOMAIN,
    INTERACTION_ERROR_DOMAIN, INTERACTION_FOLLOWUP_DOMAIN, component_claim_identity,
    component_delivery_key, interaction_delivery_key,
};
use twilight_model::id::{
    Id,
    marker::{InteractionMarker, MessageMarker},
};

#[test]
fn slash_delivery_identity_is_pinned_to_the_interaction() {
    let key = interaction_delivery_key(Id::<InteractionMarker>::new(407));
    assert_eq!(key, "interaction:407");
    assert_eq!(INTERACTION_FOLLOWUP_DOMAIN, "interaction/followup/v1");
    assert_eq!(INTERACTION_ERROR_DOMAIN, "interaction/error/v1");
}

#[test]
fn component_identity_includes_source_interaction_component_and_claim() {
    let component = ComponentId::Approval {
        thread_id: "t".into(),
        answer: ApprovalAnswer::ApproveSession,
    };
    let key = component_delivery_key(
        Id::new(407),
        Some(Id::<MessageMarker>::new(91)),
        &component,
        Some("claim-abc"),
    );
    assert_eq!(
        key,
        "v1;source-some=2:91;interaction=3:407;component=31:approval;thread=1:t;answer=1:2;;claim-some=9:claim-abc;"
    );
    assert_eq!(COMPONENT_CONFIRMATION_DOMAIN, "component/confirmation/v1");
    assert_eq!(COMPONENT_ERROR_DOMAIN, "interaction/error-component/v1");
}

#[test]
fn busy_identity_is_stable_without_a_source_message_and_separates_actions() {
    let stop = ComponentId::Busy {
        choice_id: "0123456789abcdef01234567".into(),
        action: BusyAction::Stop,
    };
    let queue = ComponentId::Busy {
        choice_id: "0123456789abcdef01234567".into(),
        action: BusyAction::Queue,
    };
    let stop_key =
        component_delivery_key(Id::new(408), None, &stop, Some("0123456789abcdef01234567"));
    assert_eq!(
        stop_key,
        "v1;source-none;interaction=3:408;component=54:busy;choice=24:0123456789abcdef01234567;action=4:stop;;claim-some=24:0123456789abcdef01234567;"
    );
    assert_ne!(
        stop_key,
        component_delivery_key(Id::new(408), None, &queue, Some("0123456789abcdef01234567"))
    );
    assert_eq!(BUSY_CONFIRMATION_DOMAIN, "component/busy-confirmation/v1");
}

#[test]
fn component_encoding_distinguishes_optional_and_delimited_fields() {
    let first = ComponentId::Input {
        thread_id: "a".into(),
        value: "b:c".into(),
    };
    let second = ComponentId::Input {
        thread_id: "a:b".into(),
        value: "c".into(),
    };
    let none = component_delivery_key(Id::new(409), None, &first, None);
    let literal_none = component_delivery_key(Id::new(409), None, &first, Some("none"));
    assert_ne!(none, literal_none);
    assert_ne!(
        none,
        component_delivery_key(Id::new(409), None, &second, None)
    );
}

#[test]
fn bound_component_identity_and_claim_are_request_scoped() {
    let first = ComponentId::BoundApproval {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c33941b38986b5fe1f0ae04ac8e5ca26".into(),
        answer: ApprovalAnswer::Approve,
    };
    let second = ComponentId::BoundApproval {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c50412aec22bb56d535dbd883397fe64".into(),
        answer: ApprovalAnswer::Approve,
    };
    let different_answer = ComponentId::BoundApproval {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c33941b38986b5fe1f0ae04ac8e5ca26".into(),
        answer: ApprovalAnswer::Reject,
    };
    let source = Some(Id::<MessageMarker>::new(91));
    assert_ne!(
        component_delivery_key(Id::new(410), source, &first, None),
        component_delivery_key(Id::new(410), source, &second, None)
    );
    assert_ne!(
        component_claim_identity(source, &first),
        component_claim_identity(source, &second)
    );
    assert_eq!(
        component_claim_identity(source, &first),
        component_claim_identity(source, &different_answer)
    );
    assert_ne!(
        component_claim_identity(source, &first),
        component_claim_identity(Some(Id::new(92)), &first)
    );
}
