use cdr_runtime::prefix_plan::{PrefixAction, plan_prefix};

#[test]
fn context_rejects_unused_or_invalid_arguments_instead_of_silently_showing_default() {
    for raw in [
        "context garbage",
        "context refresh bad",
        "context refresh 5 extra",
        "context all extra",
    ] {
        assert!(plan_prefix(raw).is_err(), "silently accepted {raw}");
    }
    assert!(matches!(
        plan_prefix("context").unwrap(),
        PrefixAction::Context {
            all_threads: false,
            refresh: false,
            ..
        }
    ));
    assert!(matches!(
        plan_prefix("ctx all").unwrap(),
        PrefixAction::Context {
            all_threads: true,
            refresh: false,
            ..
        }
    ));
    assert!(matches!(
        plan_prefix("context recent 99").unwrap(),
        PrefixAction::Context {
            refresh: true,
            limit: 30,
            ..
        }
    ));
}
