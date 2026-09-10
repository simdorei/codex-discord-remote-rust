use crate::discord_dispatch::{
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

pub async fn stage(db: &Path, name: &str) -> InboundInteractionWork {
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
    let event=serde_json::from_value(serde_json::json!({
        "application_id":"2","authorizing_integration_owners":{},"channel_id":"42",
        "data":{"id":"1","name":name,"type":1,"options":[{"name":"prompt","type":3,"value":"원래 요청"}]},
        "entitlements":[],"id":"101","locale":"ko","token":"fixture","type":2,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},"version":1
    })).unwrap();
    assert_eq!(
        dispatcher
            .dispatch(
                &event,
                tokio::time::Instant::now(),
                cdr_discord::gateway::ingress::InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    recv.recv().await.unwrap()
}
