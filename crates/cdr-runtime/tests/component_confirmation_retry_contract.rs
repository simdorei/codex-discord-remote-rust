use cdr_discord::components::{ApprovalAnswer, ComponentId};
use cdr_runtime::component_worker::{
    ConfirmationClaimState, busy_confirmation_plan, busy_ready_marker, claim_standard_action,
    confirmation_ready, read_busy_choice_state, record_confirmation_ready, send_then_clear,
    standard_confirmation_plan, standard_ready_marker,
};
use cdr_runtime::discord_dispatch::delivery_identity::component_claim_identity;
use cdr_store::claims::{
    NewBusyChoice, claim_busy_choice, cleanup_busy_choices, create_busy_choice,
};

const TTL_SECONDS: f64 = 1_800.0;

#[test]
fn standard_action_runs_once_then_replays_the_same_confirmation_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    let action_claim = "approval-claim-91";
    let ready_marker = standard_ready_marker(action_claim);
    let approval = ComponentId::Approval {
        thread_id: "thread-a".into(),
        answer: ApprovalAnswer::Approve,
    };
    let later_click = ComponentId::Approval {
        thread_id: "thread-a".into(),
        answer: ApprovalAnswer::Reject,
    };
    let first_plan = standard_confirmation_plan(&approval, action_claim).unwrap();
    let retry_plan = standard_confirmation_plan(&later_click, action_claim).unwrap();

    assert_eq!(first_plan, retry_plan);
    assert_eq!(first_plan.content, "Approval response submitted.");
    assert!(!first_plan.logical_key.contains("interaction"));

    let mut action_runs = 0;
    assert_eq!(
        claim_standard_action(&database, action_claim, &ready_marker, 10.0, TTL_SECONDS,).unwrap(),
        ConfirmationClaimState::ExecuteAction
    );
    action_runs += 1;

    // A base claim alone can mean the process died before the action completed.
    assert_eq!(
        claim_standard_action(&database, action_claim, &ready_marker, 11.0, TTL_SECONDS,).unwrap(),
        ConfirmationClaimState::ActionUnconfirmed
    );
    assert_eq!(action_runs, 1);

    record_confirmation_ready(&database, &ready_marker, 12.0, TTL_SECONDS).unwrap();
    // Each helper reopens SQLite, exercising reconstruction without process memory.
    assert_eq!(
        claim_standard_action(&database, action_claim, &ready_marker, 13.0, TTL_SECONDS,).unwrap(),
        ConfirmationClaimState::DeliverConfirmation
    );
    assert!(confirmation_ready(&database, &ready_marker, 13.0).unwrap());
    assert_eq!(action_runs, 1);
}

#[test]
fn input_replay_content_and_identity_do_not_depend_on_the_new_click() {
    let first = ComponentId::BoundInput {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c33941b38986b5fe1f0ae04ac8e5ca26".into(),
        value: "Safe".into(),
    };
    let retry = ComponentId::BoundInput {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c33941b38986b5fe1f0ae04ac8e5ca26".into(),
        value: "Fast".into(),
    };

    let first_plan = standard_confirmation_plan(&first, "input-claim-92").unwrap();
    let retry_plan = standard_confirmation_plan(&retry, "input-claim-92").unwrap();
    assert_eq!(first_plan, retry_plan);
    assert_eq!(first_plan.content, "Codex input choice submitted.");
}

#[test]
fn bound_confirmation_claim_is_request_scoped_but_answer_independent() {
    use twilight_model::id::{Id, marker::MessageMarker};

    let source = Some(Id::<MessageMarker>::new(91));
    let first = ComponentId::BoundApproval {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c33941b38986b5fe1f0ae04ac8e5ca26".into(),
        answer: ApprovalAnswer::Approve,
    };
    let different_answer = ComponentId::BoundApproval {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c33941b38986b5fe1f0ae04ac8e5ca26".into(),
        answer: ApprovalAnswer::Reject,
    };
    let different_request = ComponentId::BoundApproval {
        thread_fingerprint: "60e9aec437d0f0f6".into(),
        request_fingerprint: "c50412aec22bb56d535dbd883397fe64".into(),
        answer: ApprovalAnswer::Approve,
    };
    let first_claim = component_claim_identity(source, &first).unwrap();
    let answer_claim = component_claim_identity(source, &different_answer).unwrap();
    let request_claim = component_claim_identity(source, &different_request).unwrap();
    assert_eq!(first_claim, answer_claim);
    assert_ne!(first_claim, request_claim);
    assert_eq!(
        standard_confirmation_plan(&first, &first_claim).unwrap(),
        standard_confirmation_plan(&different_answer, &answer_claim).unwrap()
    );
    assert_ne!(
        standard_confirmation_plan(&first, &first_claim).unwrap(),
        standard_confirmation_plan(&different_request, &request_claim).unwrap()
    );

    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    assert_eq!(
        claim_standard_action(&database, &first_claim, "ready-a", 1.0, TTL_SECONDS).unwrap(),
        ConfirmationClaimState::ExecuteAction
    );
    assert_eq!(
        claim_standard_action(&database, &request_claim, "ready-b", 1.0, TTL_SECONDS).unwrap(),
        ConfirmationClaimState::ExecuteAction
    );
}

#[test]
fn busy_claim_survives_as_a_reconstructable_confirmation_after_row_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    let choice_id = create_busy_choice(
        &database,
        NewBusyChoice {
            owner_user_id: 7,
            channel_id: 8,
            target_thread_id: Some("thread-a"),
            prompt: "next",
            allow_steer: true,
            now: 10.0,
            time_to_live: TTL_SECONDS,
        },
    )
    .unwrap();
    assert!(claim_busy_choice(&database, &choice_id, 11.0).unwrap());

    let claimed = read_busy_choice_state(&database, &choice_id, 12.0)
        .unwrap()
        .unwrap();
    assert!(claimed.claimed);

    let plan = busy_confirmation_plan(&choice_id);
    let retry_plan = busy_confirmation_plan(&choice_id);
    assert_eq!(plan, retry_plan);
    assert_eq!(plan.content, "Busy action submitted.");
    assert!(!plan.logical_key.contains("interaction"));

    let ready_marker = busy_ready_marker(&choice_id, 7, 8);
    record_confirmation_ready(&database, &ready_marker, 13.0, TTL_SECONDS).unwrap();
    assert_eq!(cleanup_busy_choices(&database, 14.0).unwrap(), 0);
    assert!(
        read_busy_choice_state(&database, &choice_id, 14.0)
            .unwrap()
            .unwrap()
            .claimed
    );
    let expired_at = 10.0 + TTL_SECONDS;
    assert_eq!(cleanup_busy_choices(&database, expired_at).unwrap(), 1);
    assert!(
        read_busy_choice_state(&database, &choice_id, expired_at)
            .unwrap()
            .is_none()
    );
    assert!(confirmation_ready(&database, &ready_marker, expired_at).unwrap());
    assert!(
        !confirmation_ready(&database, &busy_ready_marker(&choice_id, 9, 8), expired_at).unwrap()
    );
    assert!(
        !confirmation_ready(&database, &busy_ready_marker(&choice_id, 7, 9), expired_at).unwrap()
    );
    assert!(!confirmation_ready(&database, &ready_marker, 13.0 + TTL_SECONDS).unwrap());
}

#[tokio::test]
async fn failed_send_keeps_buttons_and_retry_reuses_the_exact_plan_before_clearing() {
    use std::cell::{Cell, RefCell};

    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("mirror.sqlite");
    let plan = busy_confirmation_plan("0123456789abcdef01234567");
    let ready_marker = busy_ready_marker("0123456789abcdef01234567", 7, 8);
    record_confirmation_ready(&database, &ready_marker, 10.0, TTL_SECONDS).unwrap();

    let sent = RefCell::new(Vec::new());
    let buttons_cleared = Cell::new(false);
    let first: Result<(), &str> = send_then_clear(
        async {
            assert!(confirmation_ready(&database, &ready_marker, 11.0).unwrap());
            sent.borrow_mut().push(plan.clone());
            Err("Discord timeout")
        },
        async {
            buttons_cleared.set(true);
            Ok(())
        },
    )
    .await;
    assert_eq!(first, Err("Discord timeout"));
    assert!(!buttons_cleared.get());

    let retry_plan = busy_confirmation_plan("0123456789abcdef01234567");
    let retry: Result<(), &str> = send_then_clear(
        async {
            sent.borrow_mut().push(retry_plan);
            Ok(())
        },
        async {
            buttons_cleared.set(true);
            Ok(())
        },
    )
    .await;
    assert_eq!(retry, Ok(()));
    assert!(buttons_cleared.get());
    assert_eq!(sent.borrow().len(), 2);
    assert_eq!(sent.borrow()[0], sent.borrow()[1]);
}
