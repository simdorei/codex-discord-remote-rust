use crate::{
    action_executor::ActionExecutor,
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    config::{CliOptions, RuntimeConfig},
    message_worker::{self, AdmittedMessage, MessageClassification, MessageContext},
    queue_runner::QueueCoordinator,
};
use cdr_app_server::ResidentAppServer;
use cdr_discord::interaction_access::InteractionAccessPolicy;
use serde_json::json;
use std::{collections::BTreeMap, path::Path, sync::Arc, time::SystemTime};

pub(crate) struct MessageFixture {
    pub executor: ActionExecutor<AppServerTurnBackend>,
    pub server: Arc<ResidentAppServer>,
    pub queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    pub http: Arc<twilight_http::Client>,
    config: RuntimeConfig,
    attachments: reqwest::Client,
}

impl MessageFixture {
    pub async fn new(temp: &tempfile::TempDir, http: Arc<twilight_http::Client>) -> Self {
        let server = Arc::new(
            super::app_fixture::start_fake_server(temp, &temp.path().join("rpc.jsonl")).await,
        );
        Self::with_server(temp, http, server)
    }

    pub fn with_server(
        temp: &tempfile::TempDir,
        http: Arc<twilight_http::Client>,
        server: Arc<ResidentAppServer>,
    ) -> Self {
        Self::configured_server(temp, http, server, false)
    }

    pub fn with_reserve_server(
        temp: &tempfile::TempDir,
        http: Arc<twilight_http::Client>,
        server: Arc<ResidentAppServer>,
    ) -> Self {
        Self::configured_server(temp, http, server, true)
    }

    fn configured_server(
        temp: &tempfile::TempDir,
        http: Arc<twilight_http::Client>,
        server: Arc<ResidentAppServer>,
        reserve: bool,
    ) -> Self {
        let state = temp.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("../../tests/fixtures/action_state.sql"))
            .unwrap();
        let db = temp.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0)
            .unwrap();
        let mut backend = AppServerTurnBackend::new(server.clone());
        if reserve {
            crate::idle_release::install(&server, &db).unwrap();
            backend = backend.with_reserve_auto(crate::reserve_auto::ReserveAutoController::new(
                server.clone(),
                db.clone(),
            ));
        }
        let queue = Arc::new(QueueCoordinator::new(db.clone(), Arc::new(backend)));
        let executor = ActionExecutor::new(
            state,
            db,
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            queue.clone(),
        )
        .with_server(server.clone());
        let config = RuntimeConfig::from_map(
            &BTreeMap::from([
                ("DISCORD_BOT_TOKEN".into(), "fixture-token".into()),
                ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
            ]),
            CliOptions::default(),
        )
        .unwrap();
        Self {
            executor,
            server,
            queue,
            http,
            config,
            attachments: reqwest::Client::new(),
        }
    }

    pub fn context<'a>(&'a self, root: &'a Path) -> MessageContext<'a, AppServerTurnBackend> {
        MessageContext::new(
            twilight_model::id::Id::new(1),
            &self.config,
            &self.executor,
            &self.server,
            self.http.clone(),
            root,
            &self.attachments,
        )
    }

    pub fn admit(&self, content: &str) -> AdmittedMessage {
        self.admit_id(content, 801)
    }

    pub fn admit_id(&self, content: &str, message_id: u64) -> AdmittedMessage {
        let candidate = self.classify_id(content, message_id);
        message_worker::admit_message_candidate_at(candidate, SystemTime::now())
            .unwrap()
            .unwrap()
    }

    pub fn classify_id(&self, content: &str, message_id: u64) -> message_worker::MessageCandidate {
        let message = serde_json::from_value(json!({
            "attachments":[],"author":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},
            "channel_id":"42","content":content,"edited_timestamp":null,"embeds":[],"id":message_id.to_string(),
            "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
            "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
        })).unwrap();
        let policy = InteractionAccessPolicy {
            allow_all_channels: true,
            ..Default::default()
        };
        let MessageClassification::Candidate(candidate) = message_worker::classify_gateway_message(
            message,
            self.executor.mirror_db(),
            &self.config,
            &policy,
            None,
        )
        .unwrap() else {
            panic!("candidate")
        };
        candidate
            .bind_settings(&self.executor.settings_resolver())
            .unwrap()
    }
}
