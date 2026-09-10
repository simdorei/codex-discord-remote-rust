use twilight_model::channel::message::component::{ActionRow, Button, ButtonStyle, Component};

use super::{
    ComponentError, ComponentRequestId, bounded, format_input_choice, request_fingerprint,
    safe_input_value, thread_fingerprint, valid_choice_id,
};

pub fn busy_button_row(choice_id: &str, allow_steer: bool) -> Result<Component, ComponentError> {
    if !valid_choice_id(choice_id) {
        return Err(ComponentError::Invalid);
    }
    let specifications = [
        (
            if allow_steer {
                "Steer now"
            } else {
                "Steer (check)"
            },
            "steer",
            ButtonStyle::Primary,
            false,
        ),
        ("Queue next", "queue", ButtonStyle::Secondary, false),
        ("Stop reply", "stop", ButtonStyle::Danger, false),
        ("Ignore", "ignore", ButtonStyle::Secondary, false),
    ];
    let components = specifications
        .into_iter()
        .map(|(label, action, style, disabled)| {
            Ok(button(
                label,
                bounded(format!("codex_busy:{choice_id}:{action}"))?,
                style,
                disabled,
            ))
        })
        .collect::<Result<Vec<_>, ComponentError>>()?;
    Ok(row(components))
}

pub fn approval_button_row(thread_id: &str) -> Result<Component, ComponentError> {
    if thread_id.trim().is_empty() {
        return Err(ComponentError::Invalid);
    }
    let specifications = [
        ("Approve", "1", ButtonStyle::Success),
        ("Approve session", "2", ButtonStyle::Primary),
        ("Reject", "3", ButtonStyle::Danger),
        ("Cancel", "cancel", ButtonStyle::Secondary),
    ];
    let components = specifications
        .into_iter()
        .map(|(label, answer, style)| {
            Ok(button(
                label,
                bounded(format!("codex_approval:{}:{answer}", thread_id.trim()))?,
                style,
                false,
            ))
        })
        .collect::<Result<Vec<_>, ComponentError>>()?;
    Ok(row(components))
}

pub fn bound_approval_button_row(
    thread_id: &str,
    generation: u64,
    occurrence: &[u8; 16],
    request_id: ComponentRequestId<'_>,
) -> Result<Component, ComponentError> {
    let thread = thread_fingerprint(thread_id)?;
    let request = request_fingerprint(generation, occurrence, request_id);
    let specifications = [
        ("Approve", "1", ButtonStyle::Success),
        ("Approve session", "2", ButtonStyle::Primary),
        ("Reject", "3", ButtonStyle::Danger),
        ("Cancel", "cancel", ButtonStyle::Secondary),
    ];
    let components = specifications
        .into_iter()
        .map(|(label, answer, style)| {
            Ok(button(
                label,
                bounded(format!("codex_approval:v2:{thread}:{request}:{answer}"))?,
                style,
                false,
            ))
        })
        .collect::<Result<Vec<_>, ComponentError>>()?;
    Ok(row(components))
}

pub fn input_button_row(
    thread_id: &str,
    options: &[(String, String)],
) -> Result<Component, ComponentError> {
    if options.is_empty() {
        return Err(ComponentError::Invalid);
    }
    let components = options
        .iter()
        .take(5)
        .map(|(value, label)| {
            let label = label.trim().chars().take(80).collect::<String>();
            if label.is_empty() {
                return Err(ComponentError::Invalid);
            }
            Ok(button(
                &label,
                format_input_choice(thread_id, value)?,
                ButtonStyle::Primary,
                false,
            ))
        })
        .collect::<Result<Vec<_>, ComponentError>>()?;
    Ok(row(components))
}

pub fn bound_input_button_row(
    thread_id: &str,
    generation: u64,
    occurrence: &[u8; 16],
    request_id: ComponentRequestId<'_>,
    options: &[(String, String)],
) -> Result<Component, ComponentError> {
    if options.is_empty() {
        return Err(ComponentError::Invalid);
    }
    let thread = thread_fingerprint(thread_id)?;
    let request = request_fingerprint(generation, occurrence, request_id);
    let components = options
        .iter()
        .take(5)
        .map(|(value, label)| {
            let value = value.trim();
            let label = label.trim().chars().take(80).collect::<String>();
            if label.is_empty() || !safe_input_value(value) {
                return Err(ComponentError::Invalid);
            }
            Ok(button(
                &label,
                bounded(format!("codex_input:v2:{thread}:{request}:{value}"))?,
                ButtonStyle::Primary,
                false,
            ))
        })
        .collect::<Result<Vec<_>, ComponentError>>()?;
    Ok(row(components))
}

fn row(components: Vec<Component>) -> Component {
    Component::ActionRow(ActionRow {
        id: None,
        components,
    })
}

fn button(label: &str, custom_id: String, style: ButtonStyle, disabled: bool) -> Component {
    Component::Button(Button {
        id: None,
        custom_id: Some(custom_id),
        disabled,
        emoji: None,
        label: Some(label.into()),
        style,
        url: None,
        sku_id: None,
    })
}
