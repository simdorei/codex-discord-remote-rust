use cdr_discord::gateway::ingress::InteractionIngressTag;
use cdr_discord::interaction::RoutedWork;
use cdr_discord::responses::{autocomplete_response, interaction_message};
use twilight_model::{
    application::interaction::{Interaction, InteractionType},
    http::interaction::InteractionResponse,
};

use super::AutocompleteCatalog;

pub(super) fn response_for(
    interaction: &Interaction,
    tag: InteractionIngressTag,
    initial: InteractionResponse,
    work: Option<&RoutedWork>,
    autocomplete: &AutocompleteCatalog,
) -> InteractionResponse {
    let is_autocomplete = interaction.kind == InteractionType::ApplicationCommandAutocomplete;
    let mut response = if is_autocomplete {
        autocomplete_response()
    } else {
        initial
    };
    if is_autocomplete
        && tag == InteractionIngressTag::Normal
        && let Some(RoutedWork::Autocomplete(invocation)) = work
        && let Some(data) = response.data.as_mut()
    {
        data.choices = Some(autocomplete.choices(invocation));
    }
    if let Some(content) = unavailable_content(tag)
        && work.is_some()
        && !is_autocomplete
    {
        response = interaction_message(content, true);
    }
    response
}

fn unavailable_content(tag: InteractionIngressTag) -> Option<&'static str> {
    match tag {
        InteractionIngressTag::Normal => None,
        InteractionIngressTag::Busy => Some("Codex Discord is busy. Please retry shortly."),
        InteractionIngressTag::Stopping => {
            Some("Codex Discord is stopping. Please retry after restart.")
        }
    }
}
