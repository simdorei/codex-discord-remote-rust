use std::future::Future;

use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use twilight_gateway::{CloseFrame, Event, Shard, StreamExt as _};

use super::{
    activation::{ActivationWaitOutcome, ShardActivation},
    gateway_event_flags,
    ingress::GatewayIngress,
    runtime_publication::{
        GatewayIngressPublishOutcomes, publish_decoded_event, publish_receive_error,
    },
};

pub(super) async fn run_shard(
    shard: Shard,
    ingress: GatewayIngress,
    ingress_publish_outcomes: GatewayIngressPublishOutcomes,
    shutdown: watch::Receiver<bool>,
    activation: ShardActivation,
    shard_exit_sender: mpsc::Sender<u32>,
) {
    let _exit = ShardExitGuard {
        shard: shard.id().number(),
        sender: shard_exit_sender,
    };
    run_after_activation(&activation, || async move {
        run_active_shard(shard, ingress, ingress_publish_outcomes, shutdown).await;
    })
    .await;
}

struct ShardExitGuard {
    shard: u32,
    sender: mpsc::Sender<u32>,
}

impl Drop for ShardExitGuard {
    fn drop(&mut self) {
        let _ = self.sender.try_send(self.shard);
    }
}

pub(super) async fn run_after_activation<F, Fut>(activation: &ShardActivation, active: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    if activation.wait().await == ActivationWaitOutcome::Activated {
        active().await;
    }
}

async fn run_active_shard(
    mut shard: Shard,
    ingress: GatewayIngress,
    ingress_publish_outcomes: GatewayIngressPublishOutcomes,
    mut shutdown: watch::Receiver<bool>,
) {
    let shard_id = shard.id().number();
    let mut closing = false;
    loop {
        tokio::select! {
            changed = shutdown.changed(), if !closing => {
                match changed {
                    Ok(()) if *shutdown.borrow() => {
                        closing = true;
                        shard.close(CloseFrame::NORMAL);
                    }
                    Err(_) => {
                        closing = true;
                        shard.close(CloseFrame::NORMAL);
                    }
                    Ok(()) => {}
                }
            }
            item = shard.next_event(gateway_event_flags()) => {
                let Some(item) = item else { break; };
                match item {
                    Ok(event) => {
                        let terminal_close = matches!(event, Event::GatewayClose(_));
                        publish_decoded_event(
                            event,
                            Instant::now(),
                            &ingress,
                            &ingress_publish_outcomes,
                        );
                        if terminal_close && (closing || *shutdown.borrow()) { break; }
                    }
                    Err(error) => {
                        publish_receive_error(
                            shard_id,
                            error.to_string(),
                            &ingress,
                            &ingress_publish_outcomes,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "shard_tests.rs"]
mod tests;
