use cdr_discord::interaction::AutocompleteInvocation;
use cdr_runtime::discord_dispatch::AutocompleteCatalog;
use serde_json::json;

#[test]
fn current_model_list_drives_filtered_model_and_effort_choices() {
    let catalog = AutocompleteCatalog::from_model_list(&json!({"data":[
        {"model":"gpt-5.6-sol", "hidden":false, "supportedReasoningEfforts":[
            {"reasoningEffort":"high"}, {"reasoningEffort":"ultra"}
        ]},
        {"id":"gpt-5.6-terra", "supportedReasoningEfforts":[{"reasoningEffort":"medium"}]},
        {"model":"hidden", "hidden":true, "supportedReasoningEfforts":[]}
    ]}));
    let models = catalog.choices(&AutocompleteInvocation {
        command_name: "settings".into(),
        option_name: "model".into(),
        current: "sol".into(),
        selected_model: None,
    });
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "gpt-5.6-sol");

    let efforts = catalog.choices(&AutocompleteInvocation {
        command_name: "settings".into(),
        option_name: "effort".into(),
        current: String::new(),
        selected_model: Some("gpt-5.6-sol".into()),
    });
    assert_eq!(
        efforts
            .into_iter()
            .map(|choice| choice.name)
            .collect::<Vec<_>>(),
        vec!["high", "ultra"]
    );
}
