use sha2::{Digest, Sha256};

use super::ComponentError;

const REQUEST_BINDING_DOMAIN: &[u8] = b"cdr-discord/component-request-binding/v3";
const THREAD_BINDING_DOMAIN: &[u8] = b"cdr-discord/component-thread/v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentRequestId<'a> {
    String(&'a str),
    Integer(i64),
}

pub fn thread_fingerprint(thread_id: &str) -> Result<String, ComponentError> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Err(ComponentError::Invalid);
    }
    let mut hasher = Sha256::new();
    hasher.update(THREAD_BINDING_DOMAIN);
    hasher.update((thread_id.len() as u64).to_be_bytes());
    hasher.update(thread_id.as_bytes());
    Ok(hex::encode(&hasher.finalize()[..8]))
}

#[must_use]
pub fn request_fingerprint(
    generation: u64,
    occurrence: &[u8; 16],
    request_id: ComponentRequestId<'_>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(REQUEST_BINDING_DOMAIN);
    hasher.update(b"\0generation");
    hasher.update(generation.to_be_bytes());
    hasher.update(b"\0occurrence");
    hasher.update(occurrence);
    match request_id {
        ComponentRequestId::String(value) => {
            hasher.update(b"\0request-string");
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
        ComponentRequestId::Integer(value) => {
            hasher.update(b"\0request-integer");
            hasher.update(value.to_be_bytes());
        }
    }
    hex::encode(&hasher.finalize()[..16])
}
