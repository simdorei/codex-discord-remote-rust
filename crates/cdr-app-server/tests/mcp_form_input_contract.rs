use cdr_app_server::build_approval_response;
use serde_json::{Value, json};

#[test]
fn mcp_form_input_preserves_typed_values_and_validates_the_requested_schema() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/mcp-form-input.json")).unwrap();
    for mode in [Some("form"), Some("openai/form"), None] {
        for case in &cases {
            let mut params = json!({"requestedSchema":case["schema"]});
            if let Some(mode) = mode {
                params["mode"] = json!(mode);
            }
            let result = build_approval_response(
                "mcpServer/elicitation/request",
                &params,
                case["answer"].as_str().unwrap(),
            );
            if case["invalid"] == true {
                assert!(result.is_err(), "{}: {result:?}", case["name"]);
            } else {
                let action = case["action"].as_str().unwrap_or("accept");
                assert_eq!(
                    result.unwrap().0,
                    json!({
                        "action":action,"content":case.get("content").unwrap_or(&Value::Null),"_meta":null
                    }),
                    "{} / {mode:?}",
                    case["name"]
                );
            }
        }
    }
}
