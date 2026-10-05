use cdr_app_server::{Notification, ResidentAppServer, ResidentNotificationEvent};
use cdr_runtime::{
    app_backend::AppServerTurnBackend, completion_worker::run_completion_worker,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::{delivery, delivery_receipt, mirror, queue};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::{broadcast, watch};

pub struct Rig {
    pub server: Arc<ResidentAppServer>,
    pub queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    client: Arc<twilight_http::Client>,
}
impl Rig {
    pub async fn new(temp: &tempfile::TempDir, address: &str, scenario: &str) -> Self {
        let mut config = native_fixture::config(scenario);
        for (key, file) in [
            ("CDR_ACTION_RPC_LOG", "rpc.jsonl"),
            ("GOAL_TEST_LOG", "goal-rpc.log"),
            ("GOAL_TEST_ROLLOUT", "goal-rollout.jsonl"),
        ] {
            config.environment.insert(
                key.into(),
                temp.path().join(file).to_string_lossy().into_owned(),
            );
        }
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        let queue = Arc::new(QueueCoordinator::new(
            temp.path().join("mirror.sqlite"),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        ));
        let client = Arc::new(
            twilight_http::Client::builder()
                .token("fixture".into())
                .proxy(address.into(), true)
                .ratelimiter(None)
                .build(),
        );
        Self {
            server,
            queue,
            client,
        }
    }
    pub fn run(&self, receiver: broadcast::Receiver<ResidentNotificationEvent>) -> Running {
        let (stop, shutdown) = watch::channel(false);
        let task = tokio::spawn(run_completion_worker(
            receiver,
            Arc::clone(&self.server),
            Arc::clone(&self.queue),
            Arc::clone(&self.client),
            false,
            Duration::from_secs(2),
            shutdown,
        ));
        Running { stop, task }
    }
    pub fn seed(&self, target: &str, channel: i64, job: &str, turn: &str) {
        let generation = i64::try_from(self.server.generation()).unwrap();
        queue::enqueue(
            self.queue.db_path(),
            queue::NewQueueJob {
                job_id: job,
                target_thread_id: target,
                channel_id: channel,
                owner_user_id: Some(1),
                discord_message_id: None,
                app_server_generation: generation,
                prompt: "fixture",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        queue::begin_attempt(self.queue.db_path(), job, &[], generation).unwrap();
        queue::mark_running(self.queue.db_path(), job, turn, generation).unwrap();
    }
    pub fn failed(&self, target: &str, turn: &str) -> ResidentNotificationEvent {
        ResidentNotificationEvent::Notification {
            generation: self.server.generation(),
            notification: Notification {
                method: "turn/completed".into(),
                params: json!({"threadId":target,"turn":{"id":turn,"status":"failed","error":{"message":"fixture"}}}),
            },
        }
    }
    pub fn journal(&self, target: &str, turn: &str, status: &str) {
        cdr_store::observed_completion::record_for_resident(self.queue.db_path(),target,turn,
            i64::try_from(self.server.generation()).unwrap(),
            &json!({"threadId":target,"turn":{"id":turn,"status":status,"error":{"message":"fixture"}}}).to_string(),
            self.server.instance_id()).unwrap();
    }
    pub fn stored(&self, target: &str, turn: &str) -> bool {
        mirror::has_event(
            self.queue.db_path(),
            &mirror::turn_origin_marker(target, turn),
            target,
        )
        .unwrap()
    }
    pub fn delivered(&self, job: &str) -> bool {
        !delivery::list_pending(self.queue.db_path())
            .unwrap()
            .iter()
            .any(|p| p.job_id == job)
    }
    pub fn stage_unknown(&self, job: &str, channel: i64) {
        self.seed(job, channel, job, "old-turn");
        delivery::stage_queue_completion(
            self.queue.db_path(),
            job,
            "Failed\nold held evidence",
            1.0,
        )
        .unwrap();
        let key = serde_json::to_string(&(channel, "completion/v1", job, 0)).unwrap();
        delivery_receipt::begin(
            self.queue.db_path(),
            &key,
            &hex::encode(Sha256::digest(b"Failed\nold held evidence")),
        )
        .unwrap();
    }
}

pub struct Running {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl Running {
    pub async fn stop(self) {
        self.stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap();
    }
}
pub async fn wait_for(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
pub fn assert_no_execution_requests(path: &Path) {
    let log = std::fs::read_to_string(path).unwrap();
    for line in log.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(
            !matches!(
                value["method"].as_str(),
                Some("turn/start" | "turn/steer" | "thread/fork")
            ),
            "{line}"
        );
    }
}
