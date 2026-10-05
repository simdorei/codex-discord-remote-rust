use cdr_discord::components::{parse_component_id, persistent_component_claim_key};
use serde_json::json;

const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn dedicated_publication_components_preserve_exact_identity_without_native_claims() {
    for (suffix, decision) in [("a", "ApproveExact"), ("h", "KeepHeld")] {
        for revision in [1_i64, i64::MAX] {
            let custom = format!("codex_pub:v1:{ID}:{revision}:{suffix}");
            let parsed = parse_component_id(&custom).expect("dedicated publication component");
            assert_eq!(
                serde_json::to_value(&parsed).unwrap(),
                json!({
                    "RecoveryPublicationDecision": {
                        "proposal_id": ID, "revision": revision, "decision": decision
                    }
                })
            );
            assert_eq!(persistent_component_claim_key(60, &parsed), None);
        }
    }
}

#[test]
fn malformed_publication_components_do_not_enter_any_component_route() {
    for revision in [
        "0",
        "-1",
        "+1",
        "01",
        " 1",
        "1 ",
        "9223372036854775808",
        "1.0",
    ] {
        assert!(parse_component_id(&format!("codex_pub:v1:{ID}:{revision}:a")).is_none());
    }
    for custom in [
        format!("codex_pub:v2:{ID}:1:a"),
        format!("codex_pub:v1:{}:1:a", ID.to_uppercase()),
        "codex_pub:v1:abc:1:a".into(),
        format!("codex_pub:v1:{ID}:1:ApproveExact"),
        format!("codex_pub:v1:{ID}:1:s"),
        format!("codex_pub:v1:{ID}:1:a:extra"),
        "x".repeat(101),
    ] {
        assert!(parse_component_id(&custom).is_none(), "{custom}");
    }
}

#[test]
fn existing_async_and_bound_approval_routes_keep_their_distinct_claims() {
    for custom in [
        format!("codex_async:{}:0", "b".repeat(64)),
        format!("codex_approval:v2:{}:{}:1", "c".repeat(16), "d".repeat(32)),
    ] {
        let parsed = parse_component_id(&custom).unwrap();
        assert!(persistent_component_claim_key(60, &parsed).is_some());
    }
}
