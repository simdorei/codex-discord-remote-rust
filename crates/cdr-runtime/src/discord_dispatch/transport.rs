use cdr_discord::http::DiscordHttp;
use twilight_model::{
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

use super::{BoxDiscordFuture, InteractionTransport};

impl InteractionTransport for DiscordHttp {
    fn acknowledge<'a>(
        &'a self,
        id: Id<InteractionMarker>,
        token: &'a str,
        response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            DiscordHttp::acknowledge(self, id, token, response)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn update<'a>(&'a self, token: &'a str, content: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.update_initial_response(token, content)
                .await
                .map_err(|error| error.to_string())
        })
    }
}
