use super::{PrefixAction, PrefixPlanError};

const USAGE: &str = "Usage: !settings [ref] [--model <model>] [--reasoning <effort>] [--effort <effort>] [--speed <speed>]";

#[cfg(test)]
#[path = "settings_worker_contract.rs"]
mod worker_contract;

pub(super) fn plan_settings(arg: &str) -> Result<PrefixAction, PrefixPlanError> {
    let tokens = shell_words(arg)?;
    if tokens
        .iter()
        .any(|t| t == "--auto-reserve" || t.starts_with("--auto-reserve="))
    {
        return Err(PrefixPlanError::Usage(
            crate::command_plan::AUTO_RESERVE_REMOVED.into(),
        ));
    }
    let mut reference = None;
    let mut model = None;
    let mut effort = None;
    let mut speed = None;
    let mut option_query = None;
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let field = match token.as_str() {
            "--model" => Some("model"),
            "--reasoning" | "--effort" => Some("effort"),
            "--speed" => Some("speed"),
            _ => None,
        };
        if let Some(field) = field {
            let duplicate = match field {
                "model" => model.is_some(),
                "effort" => effort.is_some(),
                "speed" => speed.is_some(),
                _ => unreachable!(),
            };
            if duplicate || option_query.is_some() {
                return Err(PrefixPlanError::Usage(USAGE.into()));
            }
            if index + 1 >= tokens.len() || tokens[index + 1].starts_with("--") {
                option_query = Some(field.to_owned());
                index += 1;
                continue;
            }
            if tokens[index + 1].trim().is_empty() {
                return Err(PrefixPlanError::Usage(USAGE.into()));
            }
            match field {
                "model" => model = Some(tokens[index + 1].clone()),
                "effort" => effort = Some(tokens[index + 1].clone()),
                "speed" => speed = Some(tokens[index + 1].clone()),
                _ => unreachable!(),
            }
            index += 2;
            continue;
        }
        if token.trim().is_empty() || token.starts_with("--") || reference.is_some() {
            return Err(PrefixPlanError::Usage(USAGE.into()));
        }
        reference = Some(token.clone());
        index += 1;
    }
    if let Some(field) = option_query {
        if model.is_some() || effort.is_some() || speed.is_some() {
            return Err(PrefixPlanError::Usage(USAGE.into()));
        }
        return Ok(PrefixAction::SettingsOptions {
            reference,
            field: Some(field),
        });
    }
    Ok(PrefixAction::Settings {
        reference,
        model,
        effort,
        speed,
    })
}

fn shell_words(raw: &str) -> Result<Vec<String>, PrefixPlanError> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for character in raw.chars() {
        if escaped {
            current.push(character);
            started = true;
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if matches!(character, '\'' | '"') {
            started = true;
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            } else {
                current.push(character);
            }
        } else if character.is_whitespace() && quote.is_none() {
            if started {
                output.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            started = true;
            current.push(character);
        }
    }
    if escaped || quote.is_some() {
        return Err(PrefixPlanError::Usage(format!(
            "{USAGE}\nERROR: No closing quotation"
        )));
    }
    if started {
        output.push(current);
    }
    Ok(output)
}
