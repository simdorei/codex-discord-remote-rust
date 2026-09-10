use cdr_app_server::{RequestId, ServerRequestOccurrence};
use sha2::{Digest, Sha256};
use twilight_model::channel::message::Component;

pub const SERVER_REQUEST_PROMPT_DOMAIN: &str = "server-request/prompt/v1";
const SEMANTIC_CONTEXT: &[u8] = b"cdr-runtime/server-request-prompt/v2";

pub struct PromptIdentityMaterial<'a> {
    pub generation: u64,
    pub occurrence: &'a ServerRequestOccurrence,
    pub request_id: &'a RequestId,
    pub method: &'a str,
    pub thread_id: &'a str,
    pub text: &'a str,
    pub components: &'a [Component],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDeliveryIdentity {
    generation: u64,
    logical_key: String,
}

impl PromptDeliveryIdentity {
    pub fn new(material: &PromptIdentityMaterial<'_>) -> Result<Self, serde_json::Error> {
        let request_id = serde_json::to_string(material.request_id)?;
        let components = serde_json::to_vec(material.components)?;
        let mut digest = Sha256::new();
        digest.update(SEMANTIC_CONTEXT);
        update_length_prefixed(&mut digest, material.occurrence.as_bytes());
        update_length_prefixed(&mut digest, request_id.as_bytes());
        update_length_prefixed(&mut digest, material.method.as_bytes());
        update_length_prefixed(&mut digest, material.thread_id.as_bytes());
        update_length_prefixed(&mut digest, material.text.as_bytes());
        update_length_prefixed(&mut digest, &components);
        Ok(Self {
            generation: material.generation,
            logical_key: format!(
                "{}:{request_id}:{}",
                material.generation,
                hex::encode(digest.finalize())
            ),
        })
    }

    #[must_use]
    pub fn logical_key(&self) -> &str {
        &self.logical_key
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn chunk(&self, chunk_index: usize, total_chunks: usize) -> PromptChunkDelivery {
        PromptChunkDelivery {
            domain: SERVER_REQUEST_PROMPT_DOMAIN,
            logical_key: self.logical_key.clone(),
            chunk_index,
            attach_components: chunk_index.checked_add(1) == Some(total_chunks),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptChunkDelivery {
    pub domain: &'static str,
    pub logical_key: String,
    pub chunk_index: usize,
    pub attach_components: bool,
}

pub fn prompt_chunk_delivery(
    material: &PromptIdentityMaterial<'_>,
    chunk_index: usize,
    total_chunks: usize,
) -> Result<PromptChunkDelivery, serde_json::Error> {
    PromptDeliveryIdentity::new(material).map(|identity| identity.chunk(chunk_index, total_chunks))
}

fn update_length_prefixed(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("Rust usize values fit in u64 on supported targets")
            .to_be_bytes(),
    );
    digest.update(value);
}
