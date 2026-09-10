use std::future::pending;

use cdr_discord::gateway::ingress::ReceiveErrorIngress;
use tokio::{
    sync::{mpsc, watch},
    time::{Duration, Instant, sleep_until},
};

use super::super::DiscordRuntimeError;

const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(13);

pub(super) async fn run(
    mut receiver: mpsc::Receiver<ReceiveErrorIngress>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), DiscordRuntimeError> {
    let mut drain_deadline = shutdown
        .borrow()
        .then(|| Instant::now() + SHUTDOWN_DRAIN_TIMEOUT);
    loop {
        tokio::select! {
            biased;
            () = async {
                match drain_deadline {
                    Some(deadline) => sleep_until(deadline).await,
                    None => pending().await,
                }
            } => return Err(DiscordRuntimeError::TypedIngressDrainTimeout("receive-error")),
            changed = shutdown.changed(), if drain_deadline.is_none() => {
                if changed.is_err() || *shutdown.borrow() {
                    drain_deadline = Some(Instant::now() + SHUTDOWN_DRAIN_TIMEOUT);
                }
            }
            item = receiver.recv() => match item {
                Some(item) => {
                    eprintln!(
                        "Discord gateway receive error on shard {}: {}",
                        item.shard, item.message
                    );
                }
                None if drain_deadline.is_some() => return Ok(()),
                None => return Err(DiscordRuntimeError::TypedIngressClosed("receive-error")),
            }
        }
    }
}

#[cfg(test)]
#[path = "receive_error_tests.rs"]
mod tests;
