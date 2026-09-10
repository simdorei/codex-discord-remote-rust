use tokio::sync::{mpsc, watch};

use super::*;

fn item(shard: u32) -> ReceiveErrorIngress {
    ReceiveErrorIngress {
        shard,
        message: format!("decode-{shard}"),
    }
}

#[tokio::test]
async fn tre_lane_01_drains_errors_published_after_shutdown_begins() {
    let (sender, receiver) = mpsc::channel(1);
    let (shutdown, shutdown_rx) = watch::channel(true);
    let runner = tokio::spawn(run(receiver, shutdown_rx));
    tokio::task::yield_now().await;

    sender
        .send(item(7))
        .await
        .expect("receive-error consumer stays open during drain");
    drop(sender);
    runner.await.unwrap().unwrap();
    drop(shutdown);
}

#[tokio::test(start_paused = true)]
async fn tre_lane_02_shutdown_drain_has_a_hard_deadline() {
    let (sender, receiver) = mpsc::channel(1);
    let (_shutdown, shutdown_rx) = watch::channel(true);
    let runner = tokio::spawn(run(receiver, shutdown_rx));
    tokio::task::yield_now().await;

    tokio::time::advance(SHUTDOWN_DRAIN_TIMEOUT).await;
    tokio::task::yield_now().await;
    assert!(matches!(
        runner.await.unwrap().unwrap_err(),
        DiscordRuntimeError::TypedIngressDrainTimeout("receive-error")
    ));
    drop(sender);
}
