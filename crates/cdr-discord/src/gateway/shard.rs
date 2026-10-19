use std::future::{Future, poll_fn};
use std::pin::Pin;

use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use twilight_gateway::{CloseFrame, Event, Message, Shard, StreamExt, error::ReceiveMessageError};

use super::{
    activation::{ActivationWaitOutcome, ShardActivation},
    gateway_event_flags,
    ingress::GatewayIngress,
    runtime_publication::{
        GatewayIngressPublishOutcomes, publish_decoded_event_with_nonce, publish_receive_error,
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
            item = next_event_with_nonce(&mut shard) => {
                let Some(item) = item else { break; };
                match item {
                    Ok((event, nonce)) => {
                        let terminal_close = matches!(event, Event::GatewayClose(_));
                        publish_decoded_event_with_nonce(
                            event,
                            nonce,
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

// Retain the one field Twilight's Message model drops, while using its normal
// parser for the event itself. Polling through StreamExt's supertrait needs no
// additional dependency and has the same cancellation behavior as next_event.
async fn next_event_with_nonce<S>(
    stream: &mut S,
) -> Option<Result<(Event, Option<u64>), ReceiveMessageError>>
where
    S: StreamExt<Item = Result<Message, ReceiveMessageError>> + Unpin,
{
    loop {
        let message = match poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await? {
            Ok(message) => message,
            Err(error) => return Some(Err(error)),
        };
        match message {
            Message::Close(frame) => return Some(Ok((Event::GatewayClose(frame), None))),
            Message::Text(json) => match decode_with_nonce(json) {
                Ok(Some(event)) => return Some(Ok(event)),
                Ok(None) => {}
                Err(error) => return Some(Err(error)),
            },
        }
    }
}

fn decode_with_nonce(json: String) -> Result<Option<(Event, Option<u64>)>, ReceiveMessageError> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        t: Option<String>,
        d: Option<Nonce>,
    }
    #[derive(serde::Deserialize)]
    struct Nonce {
        nonce: Option<serde_json::Value>,
    }
    let nonce = serde_json::from_str::<Envelope>(&json)
        .ok()
        .filter(|event| event.t.as_deref() == Some("MESSAGE_CREATE"))
        .and_then(|event| event.d?.nonce)
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    twilight_gateway::parse(json, gateway_event_flags())
        .map(|event| event.map(|event| (event.into(), nonce)))
}

#[cfg(test)]
#[path = "shard_tests.rs"]
mod tests;
