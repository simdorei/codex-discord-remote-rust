use serde_json::Value;

use super::ActionError;
pub(crate) mod reserve;

fn rows(catalog: &Value) -> impl Iterator<Item = &Value> {
    catalog
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("hidden").and_then(Value::as_bool) != Some(true))
}

fn name(row: &Value) -> Option<&str> {
    ["model", "id"].into_iter().find_map(|key| {
        row.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    })
}

pub(super) fn canonical_model(catalog: &Value, requested: &str) -> Result<String, ActionError> {
    let requested = if reserve::requested(requested) {
        reserve::MODEL
    } else {
        requested.trim()
    };
    let matches = rows(catalog)
        .filter(|row| {
            name(row).is_some_and(|model| model.eq_ignore_ascii_case(requested))
                || row
                    .get("displayName")
                    .and_then(Value::as_str)
                    .is_some_and(|display| display.eq_ignore_ascii_case(requested))
        })
        .filter_map(name)
        .collect::<std::collections::BTreeSet<_>>();
    if matches.len() == 1 {
        return Ok(matches.into_iter().next().expect("one model").to_owned());
    }
    Err(ActionError::Invalid(format!(
        "model '{requested}' is not an unambiguous available model; use !settings --model to see available names"
    )))
}

pub(super) fn options(catalog: &Value, field: Option<&str>) -> Result<String, ActionError> {
    let mut models = Vec::new();
    let mut efforts = Vec::new();
    for row in rows(catalog) {
        if let Some(model) = name(row) {
            push_unique(&mut models, model);
        }
        for effort in row
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| value.get("reasoningEffort").and_then(Value::as_str))
        {
            push_unique(&mut efforts, effort);
        }
    }
    let result = match field {
        Some("model") => models.join("\n"),
        Some("effort" | "reasoning") => efforts.join("\n"),
        Some("speed") => "standard\nfast".into(),
        None => format!(
            "model: {}\neffort: {}\nspeed: standard, fast",
            models.join(", "),
            efforts.join(", ")
        ),
        Some(other) => {
            return Err(ActionError::Invalid(format!(
                "unknown settings field: {other}"
            )));
        }
    };
    if result.is_empty() {
        return Err(ActionError::Invalid(
            "model/list returned no available options".into(),
        ));
    }
    Ok(result)
}

fn push_unique(values: &mut Vec<String>, text: &str) {
    if !values.iter().any(|value| value == text) {
        values.push(text.to_owned());
    }
}

pub(super) fn effort_options(catalog: &Value, model: &str) -> Result<String, ActionError> {
    let matches: Vec<_> = rows(catalog)
        .filter(|row| name(row) == Some(model))
        .cloned()
        .collect();
    if matches.len() != 1 {
        return Err(ActionError::Invalid(
            "target model has no unambiguous catalog entry; no effort options confirmed".into(),
        ));
    }
    options(&serde_json::json!({"data": matches}), Some("effort"))
}

pub(super) fn validate_effort(
    catalog: &Value,
    model: &str,
    effort: &str,
) -> Result<(), ActionError> {
    let mut matches = rows(catalog).filter(|row| name(row) == Some(model));
    let row = matches.next().ok_or_else(|| {
        ActionError::Invalid(
            "current model is absent from model/list; effort cannot be verified".into(),
        )
    })?;
    if matches.next().is_some() {
        return Err(ActionError::Invalid(
            "current model is ambiguous; effort cannot be verified".into(),
        ));
    }
    if row
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .is_some_and(|values| {
            values
                .iter()
                .any(|v| v.get("reasoningEffort").and_then(Value::as_str) == Some(effort))
        })
    {
        return Ok(());
    }
    Err(ActionError::Invalid(
        "reasoning effort is not supported by the selected model; no settings update was sent"
            .into(),
    ))
}
