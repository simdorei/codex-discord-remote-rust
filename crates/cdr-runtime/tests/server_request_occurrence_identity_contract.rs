use cdr_app_server::{RequestId, ServerRequestOccurrence};
use cdr_discord::idempotent_message::message_nonce;
use cdr_runtime::server_request_worker::delivery_identity::{
    PromptIdentityMaterial, prompt_chunk_delivery,
};
use cdr_runtime::server_request_worker::sent_cache::SentRequestCache;
use twilight_model::id::{Id, marker::ChannelMarker};

#[test]
fn occurrence_separates_identical_prompt_nonce_and_sent_cache_identity() {
    let first = delivery(ServerRequestOccurrence::from_bytes([0x11; 16]));
    let retry = delivery(ServerRequestOccurrence::from_bytes([0x11; 16]));
    let replacement = delivery(ServerRequestOccurrence::from_bytes([0x22; 16]));

    assert_eq!(first, retry);
    assert_ne!(first, replacement);
    assert_ne!(
        first.1, replacement.1,
        "a fresh occurrence must change the Discord message nonce"
    );

    let mut cache = SentRequestCache::default();
    cache.remember(1, first.0.clone());
    assert!(cache.contains(1, &first.0));
    assert!(!cache.contains(1, &replacement.0));
}

fn delivery(occurrence: ServerRequestOccurrence) -> (String, u64) {
    let id = RequestId::Integer(42);
    let material = PromptIdentityMaterial {
        generation: 1,
        occurrence: &occurrence,
        request_id: &id,
        method: "item/commandExecution/requestApproval",
        thread_id: "thread-1",
        text: "same prompt",
        components: &[],
    };
    let delivery = prompt_chunk_delivery(&material, 0, 1).expect("delivery identity");
    let nonce = message_nonce(
        delivery.domain,
        Id::<ChannelMarker>::new(91),
        &delivery.logical_key,
        delivery.chunk_index,
    );
    (delivery.logical_key, nonce)
}
