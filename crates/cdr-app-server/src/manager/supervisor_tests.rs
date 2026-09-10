use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::watch;
use tokio::time::timeout;

use super::ResidentAppServer;
use crate::client::startup_tests::helper_config;

const RECOVERY_TIMEOUT: Duration = Duration::from_secs(2);

// SUP-C1: a quiescent current-generation death is recovered automatically.
#[tokio::test]
async fn quiescent_death_automatically_recovers_and_echoes_on_generation_two() {
    let server = Arc::new(
        ResidentAppServer::start(helper_config())
            .await
            .expect("start resident fixture"),
    );
    let (shutdown, shutdown_rx) = watch::channel(false);
    let supervisor = tokio::spawn(Arc::clone(&server).run_restart_supervisor(shutdown_rx));
    let generation_one = server
        .state
        .current_client()
        .expect("generation one client");

    generation_one.close().await.expect("kill generation one");

    timeout(RECOVERY_TIMEOUT, async {
        loop {
            if server.generation() == 2 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("SUP-C1: supervisor did not advance generation 1 to generation 2");

    assert_eq!(
        server
            .request(
                "test/echo",
                json!({"generation": 2}),
                Duration::from_secs(1),
                Some(2),
            )
            .await
            .expect("generation two echo"),
        json!({})
    );
    shutdown.send(true).expect("stop restart supervisor");
    timeout(RECOVERY_TIMEOUT, supervisor)
        .await
        .expect("restart supervisor did not stop")
        .expect("restart supervisor task failed");
    server.close().await.expect("close resident fixture");
}
