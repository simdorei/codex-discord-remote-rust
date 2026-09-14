use super::{ComponentError, valid_fingerprint};
use twilight_model::channel::message::component::{ActionRow, Button, ButtonStyle, Component};

pub fn async_choice_rows(id: &str, options: &[String]) -> Result<Vec<Component>, ComponentError> {
    if !valid_fingerprint(id, 64)
        || !(1..=25).contains(&options.len())
        || options.iter().any(|option| option.trim().is_empty())
    {
        return Err(ComponentError::Invalid);
    }
    let buttons = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            // Full labels stay in the question body. Numbering disambiguates even
            // identical/truncated labels; the ID carries only an immutable index.
            let label = format!("{}. {}", index + 1, option);
            let label = if label.chars().count() > 80 {
                format!("{}…", label.chars().take(79).collect::<String>())
            } else {
                label
            };
            Component::Button(Button {
                id: None,
                custom_id: Some(format!("codex_async:{id}:{index}")),
                disabled: false,
                emoji: None,
                label: Some(label),
                style: ButtonStyle::Primary,
                url: None,
                sku_id: None,
            })
        })
        .collect::<Vec<_>>();
    Ok(buttons
        .chunks(5)
        .map(|buttons| {
            Component::ActionRow(ActionRow {
                id: None,
                components: buttons.to_vec(),
            })
        })
        .collect())
}
