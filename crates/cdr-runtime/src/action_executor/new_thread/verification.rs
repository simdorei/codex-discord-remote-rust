use super::{ActionError, ActionExecutor, StoredIngress, TurnBackend};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn verify_new_persistence(
        &self,
        ingress: &StoredIngress,
    ) -> Result<(), ActionError> {
        if self.mirror_sync.get().is_none() {
            return Ok(());
        }
        let current = cdr_store::ingress::get(&self.mirror_db, &ingress.ingress_id)?
            .ok_or_else(|| ActionError::Invalid("new request identity disappeared".into()))?;
        let creation = current.outcome.as_ref().and_then(|value| value.get("new_creation"))
            .filter(|value| value.get("version").and_then(Value::as_u64) == Some(1)
                && value.get("origin_channel_id").and_then(Value::as_i64) == Some(current.channel_id))
            .ok_or_else(|| ActionError::Invalid("new original project evidence is unavailable; verification refused without recreation".into()))?;
        let cwd = creation
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ActionError::Invalid(
                    "new original project directory is unavailable; verification refused".into(),
                )
            })?;
        let target = current
            .target_thread_id
            .as_deref()
            .ok_or_else(|| ActionError::Invalid("new request has no recorded target".into()))?;
        let proof=current.outcome.as_ref().and_then(|v|v.get("new_verification"))
            .ok_or_else(||ActionError::Invalid(format!("new thread {target} first input has not reached durable execution; request remains saved without recreation")))?;
        let digest = proof
            .get("prompt_sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| ActionError::Invalid("new input fingerprint is unavailable".into()))?;
        let channel = proof
            .get("channel_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                ActionError::Invalid("new destination evidence is unavailable".into())
            })?;
        if proof.get("thread_id").and_then(Value::as_str) != Some(target)
            || cdr_store::mapping::mirrored_thread_id(&self.mirror_db, Some(channel))?.as_deref()
                != Some(target)
        {
            return Err(ActionError::Invalid(
                "new destination mapping changed; verification refused".into(),
            ));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        loop {
            if let Some(thread) = cdr_codex_state::CodexThreadStore::open(&self.state_db)?
                .load_thread(target, false)?
            {
                if cdr_codex_state::normalize_workspace_path(cwd)
                    != cdr_codex_state::normalize_workspace_path(&thread.cwd)
                {
                    return Err(ActionError::Invalid(format!(
                        "new thread {target} persisted in a different project; original request retained"
                    )));
                }
                if persisted_input(Path::new(&thread.rollout_path), target, digest)? {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ActionError::Invalid(format!(
                    "new thread {target} was accepted but its persisted first input could not be verified within 6 seconds; existing execution and room are preserved, no thread or prompt will be recreated"
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn persisted_input(path: &Path, target: &str, expected_digest: &str) -> Result<bool, ActionError> {
    let tail = cdr_codex_state::read_new_session_events(path, 0, Some(256))?;
    let mut identity = false;
    let mut prompt = false;
    for event in tail.events {
        let Some(payload) = event.get("payload") else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if payload.get("id").and_then(Value::as_str) != Some(target) {
                    return Err(ActionError::Invalid(
                        "new rollout belongs to a different thread".into(),
                    ));
                }
                identity = true;
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("user_message") =>
            {
                if let Some(text) = payload.get("message").and_then(Value::as_str) {
                    prompt |= matches_digest(text, expected_digest);
                }
            }
            Some("response_item")
                if payload.get("role").and_then(Value::as_str) == Some("user") =>
            {
                if let Some(content) = payload.get("content").and_then(Value::as_array) {
                    let text = content
                        .iter()
                        .filter_map(|v| v.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("");
                    prompt |= matches_digest(&text, expected_digest);
                }
            }
            _ => {}
        }
    }
    Ok(identity && prompt)
}

fn matches_digest(text: &str, expected: &str) -> bool {
    hex::encode(Sha256::digest(text.as_bytes())) == expected
}

#[cfg(test)]
#[path = "verification_tests.rs"]
mod tests;
