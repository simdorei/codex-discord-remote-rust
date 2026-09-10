use std::collections::BTreeMap;

use twilight_model::application::command::{CommandOptionChoice, CommandOptionChoiceValue};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AutocompleteCatalog {
    models: Vec<String>,
    efforts: BTreeMap<String, Vec<String>>,
}

impl AutocompleteCatalog {
    #[must_use]
    pub fn from_model_list(value: &serde_json::Value) -> Self {
        let mut catalog = Self::default();
        for row in value
            .get("data")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| row.get("hidden").and_then(serde_json::Value::as_bool) != Some(true))
        {
            let model = ["model", "id"]
                .into_iter()
                .find_map(|key| clean(row.get(key)));
            let Some(model) = model else { continue };
            push_unique(&mut catalog.models, &model);
            let efforts = catalog.efforts.entry(model).or_default();
            for effort in row
                .get("supportedReasoningEfforts")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| clean(item.get("reasoningEffort")))
            {
                push_unique(efforts, &effort);
            }
        }
        catalog
    }

    #[must_use]
    pub fn choices(
        &self,
        invocation: &cdr_discord::interaction::AutocompleteInvocation,
    ) -> Vec<CommandOptionChoice> {
        let values = if invocation.option_name == "model" {
            self.models.clone()
        } else if invocation.option_name == "effort" {
            invocation
                .selected_model
                .as_ref()
                .and_then(|model| self.efforts.get(model).cloned())
                .unwrap_or_else(|| self.all_efforts())
        } else {
            Vec::new()
        };
        let current = invocation.current.trim().to_ascii_lowercase();
        values
            .into_iter()
            .filter(|value| current.is_empty() || value.to_ascii_lowercase().contains(&current))
            .take(25)
            .map(|value| CommandOptionChoice {
                name: value.clone(),
                name_localizations: None,
                value: CommandOptionChoiceValue::String(value),
            })
            .collect()
    }

    fn all_efforts(&self) -> Vec<String> {
        self.efforts
            .values()
            .flatten()
            .fold(Vec::new(), |mut result, effort| {
                push_unique(&mut result, effort);
                result
            })
    }
}

fn clean(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}
