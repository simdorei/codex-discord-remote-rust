use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_discord::interaction_access::InteractionAccessPolicy;
use cdr_runtime::discord_dispatch::{BoxDiscordFuture, InteractionTransport};
use serde_json::json;
use twilight_model::application::interaction::Interaction;
use twilight_model::http::interaction::InteractionResponse;
use twilight_model::id::{Id, marker::InteractionMarker};

pub struct InspectingTransport {
    database: PathBuf,
    pub seen_at_ack: Mutex<Vec<cdr_store::ingress::StoredIngress>>,
    pub responses: Mutex<Vec<InteractionResponse>>,
    fail_ack: AtomicBool,
}

impl InspectingTransport {
    pub fn new(database: &Path) -> Self {
        Self {
            database: database.to_owned(),
            seen_at_ack: Mutex::new(Vec::new()),
            responses: Mutex::new(Vec::new()),
            fail_ack: AtomicBool::new(false),
        }
    }

    #[allow(
        dead_code,
        reason = "shared support; only failure-path integration targets toggle ACKs"
    )]
    pub fn set_fail_ack(&self, fail: bool) {
        self.fail_ack.store(fail, Ordering::SeqCst);
    }
}

impl InteractionTransport for InspectingTransport {
    fn acknowledge<'a>(
        &'a self,
        id: Id<InteractionMarker>,
        _token: &'a str,
        response: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async move {
            let row =
                cdr_store::ingress::by_origin(&self.database, i64::try_from(id.get()).unwrap())
                    .unwrap();
            if let Some(row) = row {
                self.seen_at_ack.lock().unwrap().push(row);
            }
            self.responses.lock().unwrap().push(response.clone());
            if self.fail_ack.load(Ordering::SeqCst) {
                Err("simulated Discord ACK failure".into())
            } else {
                Ok(())
            }
        })
    }

    fn update<'a>(&'a self, _token: &'a str, _content: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

pub fn policy() -> InteractionAccessPolicy {
    InteractionAccessPolicy {
        allowed_channel_ids: BTreeSet::from([10]),
        allowed_user_ids: BTreeSet::from([20]),
        mirrored_channel_ids: BTreeSet::new(),
        allow_all_channels: false,
    }
}

#[allow(
    dead_code,
    reason = "shared support; not every integration target uses each fixture"
)]
pub fn command(id: u64, token: &str) -> Interaction {
    serde_json::from_value(json!({
        "application_id":"2", "authorizing_integration_owners":{}, "channel_id":"10",
        "data":{"id":"3","name":"help","type":1}, "entitlements":[],
        "id":id.to_string(), "locale":"en-US", "token":token, "type":2,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"tester"},
        "version":1
    }))
    .unwrap()
}

pub fn busy_component(id: u64, token: &str, choice_id: &str, action: &str) -> Interaction {
    serde_json::from_value(json!({
        "application_id":"2", "authorizing_integration_owners":{}, "channel_id":"10",
        "data":{"component_type":2,"custom_id":format!("codex_busy:{choice_id}:{action}")},
        "entitlements":[], "id":id.to_string(), "locale":"en-US", "token":token, "type":3,
        "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"20","username":"tester"},
        "version":1
    }))
    .unwrap()
}

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
