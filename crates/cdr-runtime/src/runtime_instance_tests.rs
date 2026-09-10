use std::time::Duration;

use tokio::sync::watch;
use tokio::time::timeout;

use super::run_heartbeat;

#[tokio::test]
async fn wsu_10_aborted_heartbeat_removes_its_marker() {
    let temp = tempfile::tempdir().expect("temporary runtime directory");
    let path = temp.path().join("heartbeat");
    let (_shutdown, receiver) = watch::channel(false);
    let heartbeat_path = path.clone();
    let heartbeat = tokio::spawn(run_heartbeat(heartbeat_path, receiver));

    timeout(Duration::from_secs(1), async {
        while !path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("heartbeat marker is written");

    heartbeat.abort();
    let _ = heartbeat.await;

    assert!(!path.exists(), "aborted heartbeat must remove its marker");
}
