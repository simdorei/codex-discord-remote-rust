use cdr_discord::components::parse_component_id;

#[test]
fn async_choice_ids_are_routed_separately_from_blocking_input() {
    let id = format!("codex_async:{}:1", "a".repeat(64));
    assert!(
        parse_component_id(&id).is_some(),
        "actual Discord custom_id must reach a handler"
    );
    assert!(parse_component_id(&format!("codex_async:{}:99", "a".repeat(64))).is_none());
    assert!(parse_component_id("codex_async:forged:1").is_none());
}

#[test]
fn every_option_survives_serialization_with_its_exact_index() {
    use cdr_discord::components::{ComponentId, async_choice_rows};
    let id = "b".repeat(64);
    let options = (0..25)
        .map(|_| "같은 선택지".repeat(20))
        .collect::<Vec<_>>();
    let rows = async_choice_rows(&id, &options).unwrap();
    let wire = serde_json::to_value(&rows).unwrap();
    let buttons = wire
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row["components"].as_array().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(buttons.len(), 25);
    for (index, button) in buttons.iter().enumerate() {
        assert_eq!(
            parse_component_id(button["custom_id"].as_str().unwrap()),
            Some(ComponentId::AsyncChoice {
                question_id: id.clone(),
                option: index
            })
        );
        assert!(button["label"].as_str().unwrap().chars().count() <= 80);
    }
    assert!(async_choice_rows(&id, &vec!["X".into(); 26]).is_err());
}
