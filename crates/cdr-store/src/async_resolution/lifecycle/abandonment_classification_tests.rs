use super::ordinary;
use serde_json::json;

const JOB: &str = "b3d5a1a3-5c3e-4764-967b-0cef767efde9";
const ID: &str = "cccccccccccccccccccccccccccccccc";

#[test]
fn exact_discard_command_is_non_control_without_granting_start_authority() {
    assert!(ordinary(
        &json!({"version":1,"plan":{"Execute":{"DiscardRequest":{"job_id":JOB}}}})
    ));
    for fields in [
        json!({"job_id":"bad"}),
        json!({"job_id":JOB.to_uppercase()}),
        json!({"job_id":JOB,"extra":true}),
        json!({"request_id":JOB}),
    ] {
        assert!(!ordinary(
            &json!({"version":1,"plan":{"Execute":{"DiscardRequest":fields}}})
        ));
    }
}

#[test]
fn exact_discard_choices_do_not_alias_publication_or_native_approval() {
    for decision in ["AbandonOnly", "KeepHeld"] {
        assert!(ordinary(
            &json!({"version":1,"work":{"Component":{"RecoveryAbandonDecision":{
                "proposal_id":ID,"revision":1,"decision":decision
            }}}})
        ));
    }
    for fields in [
        json!({"proposal_id":ID,"revision":1,"decision":"ApproveExact"}),
        json!({"proposal_id":ID,"revision":0,"decision":"AbandonOnly"}),
        json!({"proposal_id":"bad","revision":1,"decision":"AbandonOnly"}),
        json!({"proposal_id":ID,"revision":1,"decision":"AbandonOnly","extra":true}),
    ] {
        assert!(!ordinary(
            &json!({"version":1,"work":{"Component":{"RecoveryAbandonDecision":fields}}})
        ));
    }
}

#[test]
fn mixed_lifecycle_discriminators_and_native_controls_still_remain_held() {
    assert!(!ordinary(&json!({"version":1,
        "plan":{"Execute":{"DiscardRequest":{"job_id":JOB}}},
        "lifecycle_binding":{"target":"target","route":"Explicit","command":{"Stop":{"reference":"target"}}}
    })));
    assert!(!ordinary(&json!({"version":1,"work":{"Component":{
        "RecoveryAbandonDecision":{"proposal_id":ID,"revision":1,"decision":"AbandonOnly"},
        "Approval":{"thread_id":"target","answer":"Approve"}
    }}})));
    assert!(!ordinary(
        &json!({"version":1,"plan":{"Execute":{"Archive":{"reference":"target"}}}})
    ));
}
