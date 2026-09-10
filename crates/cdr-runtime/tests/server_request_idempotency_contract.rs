use cdr_app_server::{RequestId, ServerRequestOccurrence};
use cdr_discord::components::approval_button_row;
use cdr_discord::idempotent_message::message_nonce;
use cdr_runtime::server_request_worker::delivery_identity::{
    PromptIdentityMaterial, SERVER_REQUEST_PROMPT_DOMAIN, prompt_chunk_delivery,
};
use cdr_runtime::server_request_worker::sent_cache::{MAX_COMMITTED_REQUESTS, SentRequestCache};
use twilight_model::channel::message::Component;
use twilight_model::id::{Id, marker::ChannelMarker};

fn channel(raw: u64) -> Id<ChannelMarker> {
    Id::new(raw)
}

const fn occurrence(byte: u8) -> ServerRequestOccurrence {
    ServerRequestOccurrence::from_bytes([byte; 16])
}

fn nonce_prefix(
    generation: u64,
    id: &RequestId,
    text: &str,
    components: &[Component],
    channel_id: Id<ChannelMarker>,
    attempted_chunks: usize,
    total_chunks: usize,
) -> Vec<u64> {
    let occurrence = occurrence(0x11);
    let material = PromptIdentityMaterial {
        generation,
        occurrence: &occurrence,
        request_id: id,
        method: "item/commandExecution/requestApproval",
        thread_id: "thread-1",
        text,
        components,
    };
    (0..attempted_chunks)
        .map(|part| {
            let delivery =
                prompt_chunk_delivery(&material, part, total_chunks).expect("delivery identity");
            message_nonce(
                delivery.domain,
                channel_id,
                &delivery.logical_key,
                delivery.chunk_index,
            )
        })
        .collect()
}

fn logical_key(id: &RequestId, method: &str, thread_id: &str, text: &str) -> String {
    let occurrence = occurrence(0x11);
    let material = PromptIdentityMaterial {
        generation: 1,
        occurrence: &occurrence,
        request_id: id,
        method,
        thread_id,
        text,
        components: &[],
    };
    prompt_chunk_delivery(&material, 0, 1)
        .expect("delivery identity")
        .logical_key
}

#[test]
fn retry_and_recovery_reuse_the_same_semantic_request_identity() {
    let id = RequestId::String("approval-request-42".into());
    let first = nonce_prefix(7, &id, "deploy alpha", &[], channel(91), 2, 2);
    let retry = nonce_prefix(7, &id, "deploy alpha", &[], channel(91), 2, 2);

    assert_eq!(first, retry);
    let material = PromptIdentityMaterial {
        generation: 7,
        occurrence: &occurrence(0x11),
        request_id: &id,
        method: "item/commandExecution/requestApproval",
        thread_id: "thread-1",
        text: "private deploy alpha detail",
        components: &[],
    };
    let delivery = prompt_chunk_delivery(&material, 0, 1).expect("delivery identity");
    assert_eq!(delivery.domain, SERVER_REQUEST_PROMPT_DOMAIN);
    assert!(
        delivery
            .logical_key
            .starts_with("7:\"approval-request-42\":")
    );
    assert!(!delivery.logical_key.contains("private deploy alpha detail"));
}

#[test]
fn chunks_channels_generations_and_request_id_types_are_separate() {
    let numeric = RequestId::Integer(42);
    let string = RequestId::String("42".into());
    let numeric_chunks = nonce_prefix(7, &numeric, "prompt", &[], channel(91), 2, 2);

    assert_ne!(numeric_chunks[0], numeric_chunks[1]);
    assert_ne!(
        numeric_chunks,
        nonce_prefix(7, &string, "prompt", &[], channel(91), 2, 2)
    );
    assert_ne!(
        numeric_chunks,
        nonce_prefix(7, &numeric, "prompt", &[], channel(92), 2, 2)
    );
    assert_ne!(
        numeric_chunks,
        nonce_prefix(8, &numeric, "prompt", &[], channel(91), 2, 2)
    );
}

#[test]
fn semantic_material_separates_reused_process_local_ids() {
    let id = RequestId::Integer(42);
    let buttons = approval_button_row("thread-1").expect("approval components");
    let base = nonce_prefix(1, &id, "deploy alpha", &[], channel(91), 1, 1);

    assert_ne!(
        base,
        nonce_prefix(1, &id, "deploy beta", &[], channel(91), 1, 1)
    );
    assert_ne!(
        base,
        nonce_prefix(1, &id, "deploy alpha", &[buttons], channel(91), 1, 1)
    );
    assert_ne!(
        logical_key(&id, "method-a", "thread-1", "deploy alpha"),
        logical_key(&id, "method-b", "thread-1", "deploy alpha")
    );
    assert_ne!(
        logical_key(&id, "method-a", "thread-1", "deploy alpha"),
        logical_key(&id, "method-a", "thread-2", "deploy alpha")
    );
}

#[test]
fn partial_replay_reuses_each_already_attempted_chunk() {
    let id = RequestId::Integer(-7);
    let full_attempt = nonce_prefix(7, &id, "prompt", &[], channel(91), 3, 3);
    let replay_after_partial_failure = nonce_prefix(7, &id, "prompt", &[], channel(91), 2, 3);

    assert_eq!(replay_after_partial_failure, full_attempt[..2]);
}

#[test]
fn components_are_reserved_for_the_final_chunk() {
    let id = RequestId::Integer(7);
    let material = PromptIdentityMaterial {
        generation: 7,
        occurrence: &occurrence(0x11),
        request_id: &id,
        method: "item/commandExecution/requestApproval",
        thread_id: "thread-1",
        text: "prompt",
        components: &[],
    };
    let first = prompt_chunk_delivery(&material, 0, 3).expect("first chunk");
    let middle = prompt_chunk_delivery(&material, 1, 3).expect("middle chunk");
    let final_chunk = prompt_chunk_delivery(&material, 2, 3).expect("final chunk");

    assert!(!first.attach_components);
    assert!(!middle.attach_components);
    assert!(final_chunk.attach_components);
}

#[test]
fn committed_request_cache_is_generation_scoped_and_bounded() {
    let mut cache = SentRequestCache::default();
    for index in 0..=MAX_COMMITTED_REQUESTS {
        cache.remember(7, format!("request-{index}"));
    }

    assert_eq!(cache.len(), MAX_COMMITTED_REQUESTS);
    assert!(!cache.contains(7, "request-0"));
    assert!(cache.contains(7, &format!("request-{MAX_COMMITTED_REQUESTS}")));

    cache.remember(8, "new-generation".into());
    assert_eq!(cache.len(), 1);
    assert!(!cache.contains(8, "request-1"));
    assert!(cache.contains(8, "new-generation"));
}
