use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use cdr_discord::gateway::ingress::InteractionIngressTag;
use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_runtime::discord_dispatch::{
    BoxDiscordFuture, DispatchOutcome, InteractionClaimCache, InteractionDispatcher,
    InteractionTransport,
};
use serde_json::json;
use tokio::{sync::mpsc, time::Instant};
use twilight_model::{
    application::interaction::Interaction,
    http::interaction::{InteractionResponse, InteractionResponseType},
    id::{Id, marker::InteractionMarker},
};

#[path = "support/interaction_dispatch.rs"]
mod dispatch_support;
use dispatch_support::DispatchDatabase;

#[derive(Default)]
struct RecordingTransport {
    responses: Mutex<Vec<InteractionResponse>>,
}

impl InteractionTransport for RecordingTransport {
    fn acknowledge<'a>(
        &'a self,
        _id: Id<InteractionMarker>,
        _token: &'a str,
        response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.responses.lock().unwrap().push(response.clone());
            Ok(())
        })
    }

    fn update<'a>(&'a self, _token: &'a str, _content: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn command(id: u64) -> Interaction {
    serde_json::from_value(json!({
        "application_id":"2",
        "authorizing_integration_owners":{},
        "channel_id":"10",
        "data":{"id":"3","name":"help","type":1},
        "entitlements":[],
        "id":id.to_string(),
        "locale":"en-US",
        "token":format!("token-{id}"),
        "type":2,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"tester"},
        "version":1
    }))
    .unwrap()
}

fn autocomplete(id: u64) -> Interaction {
    serde_json::from_value(json!({
        "application_id":"2",
        "authorizing_integration_owners":{},
        "channel_id":"10",
        "data":{"id":"3","name":"settings","type":1,"options":[
            {"name":"model","type":3,"value":"gpt","focused":true}
        ]},
        "entitlements":[],
        "id":id.to_string(),
        "locale":"en-US",
        "token":format!("token-{id}"),
        "type":4,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"tester"},
        "version":1
    }))
    .unwrap()
}

fn policy() -> InteractionAccessPolicy {
    InteractionAccessPolicy {
        allowed_channel_ids: BTreeSet::from([10]),
        allowed_user_ids: BTreeSet::from([20]),
        mirrored_channel_ids: BTreeSet::new(),
        allow_all_channels: false,
    }
}

#[tokio::test]
async fn tic_01_busy_and_stopping_acknowledge_without_exposing_work() {
    for (id, tag, expected) in [
        (801, InteractionIngressTag::Busy, "busy"),
        (802, InteractionIngressTag::Stopping, "stopping"),
    ] {
        let database = DispatchDatabase::new();
        let transport = Arc::new(RecordingTransport::default());
        let (sender, mut receiver) = mpsc::channel(1);
        let dispatcher = InteractionDispatcher::new(
            Arc::clone(&transport),
            policy(),
            false,
            sender,
            database.path(),
        );

        assert_eq!(
            dispatcher
                .dispatch(&command(id), Instant::now(), tag)
                .await
                .unwrap(),
            DispatchOutcome::RespondedWithoutWork
        );
        let responses = transport.responses.lock().unwrap();
        assert_eq!(responses.len(), 1);
        let content = responses[0]
            .data
            .as_ref()
            .and_then(|data| data.content.as_deref())
            .unwrap();
        assert!(content.to_ascii_lowercase().contains(expected));
        assert!(receiver.try_recv().is_err());
    }
}

#[tokio::test]
async fn tic_02_busy_autocomplete_keeps_a_protocol_valid_empty_response() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(RecordingTransport::default());
    let (sender, mut receiver) = mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    );

    assert_eq!(
        dispatcher
            .dispatch(
                &autocomplete(803),
                Instant::now(),
                InteractionIngressTag::Busy,
            )
            .await
            .unwrap(),
        DispatchOutcome::RespondedWithoutWork
    );
    let responses = transport.responses.lock().unwrap();
    assert_eq!(
        responses[0].kind,
        InteractionResponseType::ApplicationCommandAutocompleteResult
    );
    assert!(
        responses[0]
            .data
            .as_ref()
            .and_then(|data| data.choices.as_ref())
            .is_some_and(Vec::is_empty)
    );
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn tic_03_normal_and_reserved_dispatchers_share_one_claim_cache() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(RecordingTransport::default());
    let claims = InteractionClaimCache::new(8);
    let (sender, mut receiver) = mpsc::channel(1);
    let normal = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender.clone(),
        database.path(),
    )
    .with_claim_cache(claims.clone());
    let reserved = InteractionDispatcher::new(
        Arc::clone(&transport),
        policy(),
        false,
        sender,
        database.path(),
    )
    .with_claim_cache(claims);

    assert_eq!(
        normal
            .dispatch(&command(804), Instant::now(), InteractionIngressTag::Normal,)
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    assert_eq!(receiver.recv().await.unwrap().interaction_id.get(), 804);
    assert_eq!(
        reserved
            .dispatch(&command(804), Instant::now(), InteractionIngressTag::Busy,)
            .await
            .unwrap(),
        DispatchOutcome::Duplicate
    );
    assert_eq!(transport.responses.lock().unwrap().len(), 1);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn tic_07_denied_busy_autocomplete_still_uses_autocomplete_protocol_response() {
    let database = DispatchDatabase::new();
    let transport = Arc::new(RecordingTransport::default());
    let (sender, mut receiver) = mpsc::channel(1);
    let mut denied = policy();
    denied.allowed_user_ids = BTreeSet::from([999]);
    let dispatcher = InteractionDispatcher::new(
        Arc::clone(&transport),
        denied,
        false,
        sender,
        database.path(),
    );

    assert_eq!(
        dispatcher
            .dispatch(
                &autocomplete(805),
                Instant::now(),
                InteractionIngressTag::Busy,
            )
            .await
            .unwrap(),
        DispatchOutcome::RespondedWithoutWork
    );
    let responses = transport.responses.lock().unwrap();
    assert_eq!(
        responses[0].kind,
        InteractionResponseType::ApplicationCommandAutocompleteResult
    );
    assert!(
        responses[0]
            .data
            .as_ref()
            .and_then(|data| data.choices.as_ref())
            .is_some_and(Vec::is_empty)
    );
    assert!(receiver.try_recv().is_err());
}
