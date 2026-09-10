use cdr_runtime::discord_dispatch::{
    BoxDiscordFuture, DispatchOutcome, InboundInteractionWork, InteractionDispatcher,
    InteractionTransport,
};
use std::{path::Path, sync::Arc};
use twilight_model::{
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

struct Ack;
impl InteractionTransport for Ack {
    fn acknowledge<'a>(
        &'a self,
        _: Id<InteractionMarker>,
        _: &'a str,
        _: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn update<'a>(&'a self, _: &'a str, _: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

pub async fn click(db: &Path, channel: u64, user: u64, custom_id: &str) -> InboundInteractionWork {
    let (send, mut recv) = tokio::sync::mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::new(Ack),
        cdr_discord::interaction_access::InteractionAccessPolicy {
            allow_all_channels: true,
            ..Default::default()
        },
        false,
        send,
        db,
    );
    let interaction = serde_json::from_value(serde_json::json!({
        "application_id":"2", "authorizing_integration_owners":{}, "channel_id":channel.to_string(),
        "data":{"component_type":2,"custom_id":custom_id}, "entitlements":[],
        "id":"201", "locale":"en-US", "token":"fixture", "type":3,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":user.to_string(),"username":"fixture"},
        "message":{"attachments":[],"author":{"avatar":null,"bot":true,"discriminator":"0001","id":"2","username":"fixture"},
          "channel_id":channel.to_string(),"content":"Approval required","edited_timestamp":null,"embeds":[],"id":"301",
          "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,"timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0},
        "version":1
    })).unwrap();
    assert_eq!(
        dispatcher
            .dispatch(
                &interaction,
                tokio::time::Instant::now(),
                cdr_discord::gateway::ingress::InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    recv.recv().await.unwrap()
}
