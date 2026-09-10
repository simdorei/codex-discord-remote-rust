use super::{ServerPrompt, ServerPromptError, component_request_id};
use cdr_app_server::ServerRequest;
use cdr_discord::components::bound_input_button_row;

pub(super) fn input_prompt(
    request: &ServerRequest,
    thread_id: String,
    generation: u64,
) -> Result<ServerPrompt, ServerPromptError> {
    let questions = cdr_app_server::validate_input_questions(&request.params)
        .map_err(|_| ServerPromptError::InvalidQuestions)?;
    let mut sections = Vec::new();
    for (index, question) in questions.iter().enumerate() {
        if question
            .get("isSecret")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            return Err(ServerPromptError::SecretInput);
        }
        let id = question
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(ServerPromptError::InvalidQuestions)?;
        let title = question
            .get("question")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(ServerPromptError::InvalidQuestions)?;
        let mut lines = vec![format!("{}. {title} [{id}]", index + 1)];
        for (option_index, option) in input_options(question)?.iter().enumerate() {
            lines.push(format!(
                "   {}. {}{}",
                option_index + 1,
                option.1,
                option_description(question, option_index)
            ));
        }
        sections.push(lines.join("\n"));
    }
    let components = if questions.len() == 1 {
        let options = input_options(&questions[0])?;
        if options.is_empty() {
            Vec::new()
        } else {
            vec![bound_input_button_row(
                &thread_id,
                generation,
                request.occurrence.as_bytes(),
                component_request_id(&request.id),
                &options,
            )?]
        }
    } else {
        Vec::new()
    };
    let reply_help = if questions.len() == 1 {
        "Reply in this channel with an option number, label, or free text.".to_owned()
    } else {
        let example = questions
            .iter()
            .filter_map(|question| question.get("id").and_then(serde_json::Value::as_str))
            .map(|id| format!("{id}=1"))
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "Reply as question_id=value pairs, for example: {example}. Use | for multiple selections."
        )
    };
    Ok(ServerPrompt {
        text: format!(
            "Codex needs input\nthread: {thread_id}\n{}\n{reply_help}\n{}",
            sections.join("\n"),
            super::text_binding::example(request, generation)
        ),
        thread_id,
        components,
    })
}

fn input_options(question: &serde_json::Value) -> Result<Vec<(String, String)>, ServerPromptError> {
    Ok(cdr_app_server::input_option_labels(question)
        .map_err(|_| ServerPromptError::InvalidQuestions)?
        .into_iter()
        .enumerate()
        .map(|(index, label)| ((index + 1).to_string(), label.to_owned()))
        .collect())
}

fn option_description(question: &serde_json::Value, index: usize) -> String {
    question
        .get("options")
        .and_then(serde_json::Value::as_array)
        .and_then(|options| options.get(index))
        .and_then(|option| option.get("description"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(String::new, |value| format!(" — {value}"))
}
