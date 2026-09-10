use std::future::Future;
use std::time::Duration;

use crate::text::split_delivery_chunks;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryPolicy {
    pub retry_delays: Vec<Duration>,
    pub chunk_markers: bool,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            retry_delays: vec![Duration::from_millis(750), Duration::from_secs(2)],
            chunk_markers: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryFailure<E> {
    pub part: usize,
    pub total_parts: usize,
    pub attempts: usize,
    pub source: E,
}

pub async fn deliver_text<E, F, Fut>(
    text: &str,
    policy: &DeliveryPolicy,
    mut send: F,
) -> Result<usize, DeliveryFailure<E>>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    deliver_text_indexed(text, policy, |_, chunk| send(chunk)).await
}

pub async fn deliver_text_indexed<E, F, Fut>(
    text: &str,
    policy: &DeliveryPolicy,
    send: F,
) -> Result<usize, DeliveryFailure<E>>
where
    F: FnMut(usize, String) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let chunks = split_delivery_chunks(text, policy.chunk_markers);
    deliver_chunks_indexed(chunks, policy, send).await
}

pub async fn deliver_chunks_indexed<E, F, Fut>(
    chunks: Vec<String>,
    policy: &DeliveryPolicy,
    mut send: F,
) -> Result<usize, DeliveryFailure<E>>
where
    F: FnMut(usize, String) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let total_parts = chunks.len();
    for (index, chunk) in chunks.into_iter().enumerate() {
        for attempt in 0..=policy.retry_delays.len() {
            match send(index, chunk.clone()).await {
                Ok(()) => break,
                Err(source) if attempt == policy.retry_delays.len() => {
                    return Err(DeliveryFailure {
                        part: index + 1,
                        total_parts,
                        attempts: attempt + 1,
                        source,
                    });
                }
                Err(_) => tokio::time::sleep(policy.retry_delays[attempt]).await,
            }
        }
    }
    Ok(total_parts)
}
