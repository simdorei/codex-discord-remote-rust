use cdr_runtime::{
    bridge_state::BridgeState,
    discord_dispatch::{
        BoxDiscordFuture, DispatchOutcome, InteractionDispatcher, InteractionTransport,
    },
    settings_binding::SettingsTargetResolver,
};
use std::sync::{Arc, Mutex};
use twilight_model::{
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

#[derive(Default)]
struct Transport(Mutex<Vec<InteractionResponse>>);
impl InteractionTransport for Transport {
    fn acknowledge<'a>(
        &'a self,
        _: Id<InteractionMarker>,
        _: &'a str,
        response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            self.0.lock().unwrap().push(response.clone());
            Ok(())
        })
    }
    fn update<'a>(&'a self, _: &'a str, _: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { panic!("unadmitted command must not PATCH a deferred response") })
    }
}

#[tokio::test]
async fn invalid_settings_target_is_one_command_error_not_fatal_custody_failure() {
    for explicit in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        let db = root.path().join("mirror.sqlite");
        cdr_store::schema::open_initialized(&db).unwrap();
        let bridge = Arc::new(BridgeState::new(root.path().join("bridge.json")));
        let transport = Arc::new(Transport::default());
        let (send, mut recv) = tokio::sync::mpsc::channel(1);
        let dispatcher = InteractionDispatcher::new(
            transport.clone(),
            cdr_discord::interaction_access::InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            false,
            send,
            &db,
        )
        .with_settings_resolver(SettingsTargetResolver::new(
            state,
            db.clone(),
            bridge.clone(),
        ));
        let mut options = vec![serde_json::json!({"name":"model","type":3,"value":"Model B"})];
        if explicit {
            options.push(serde_json::json!({"name":"ref","type":3,"value":"missing-thread"}));
        }
        let event = serde_json::from_value(serde_json::json!({
            "application_id":"2","authorizing_integration_owners":{},"channel_id":"42",
            "data":{"id":"1","name":"settings","type":1,"options":options},
            "entitlements":[],"id":"201","locale":"en-US","token":"fixture","type":2,
            "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},"version":1
        })).unwrap();
        let result = dispatcher
            .dispatch(
                &event,
                tokio::time::Instant::now(),
                cdr_discord::gateway::ingress::InteractionIngressTag::Normal,
            )
            .await;
        // Rejection is durably staged for the existing worker response path.
        assert_eq!(result.unwrap(), DispatchOutcome::Queued);
        let responses = transport.0.lock().unwrap();
        assert_eq!(responses.len(), 1);
        let _work = recv.try_recv().unwrap();
        let record = cdr_store::ingress::get(&db, "interaction:201")
            .unwrap()
            .unwrap();
        assert!(
            record.payload["request_rejection"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
        assert!(record.payload["settings_binding"].is_null());
        assert!(record.target_thread_id.is_none());
        assert!(!bridge.path().exists());
    }
}
