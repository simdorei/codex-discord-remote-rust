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

pub async fn stage(db: &Path, name: &str) -> InboundInteractionWork {
    let options = if matches!(name, "ask" | "interview") {
        serde_json::json!([{"name":"prompt","type":3,"value":"원래 요청"}])
    } else {
        serde_json::json!([])
    };
    stage_options(db, name, options).await
}

pub async fn stage_options(
    db: &Path,
    name: &str,
    options: serde_json::Value,
) -> InboundInteractionWork {
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
    )
    .with_settings_resolver(cdr_runtime::settings_binding::SettingsTargetResolver::new(
        db.parent().unwrap().join("state.sqlite"),
        db.to_path_buf(),
        Arc::new(cdr_runtime::bridge_state::BridgeState::new(
            db.parent().unwrap().join("bridge.json"),
        )),
    ));
    let event = serde_json::from_value(serde_json::json!({
        "application_id":"2","authorizing_integration_owners":{},"channel_id":"42",
        "data":{"id":"1","name":name,"type":1,"options":options},
        "entitlements":[],"id":"101","locale":"en-US","token":"fixture","type":2,
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
