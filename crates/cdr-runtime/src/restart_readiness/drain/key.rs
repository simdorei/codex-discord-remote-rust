use super::DrainGateError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrainFenceKey {
    runtime_id: String,
    process_identity: String,
    nonce: String,
}

impl DrainFenceKey {
    pub fn new(
        runtime_id: impl Into<String>,
        process_identity: impl Into<String>,
        nonce: impl Into<String>,
    ) -> Result<Self, DrainGateError> {
        let value = Self {
            runtime_id: runtime_id.into(),
            process_identity: process_identity.into(),
            nonce: nonce.into(),
        };
        if !valid_token(&value.runtime_id)
            || !valid_process_identity(&value.process_identity)
            || !valid_token(&value.nonce)
        {
            return Err(DrainGateError::InvalidKey);
        }
        Ok(value)
    }

    #[must_use]
    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    #[must_use]
    pub fn process_identity(&self) -> &str {
        &self.process_identity
    }

    #[must_use]
    pub fn nonce(&self) -> &str {
        &self.nonce
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_process_identity(value: &str) -> bool {
    let Some((pid, ticks)) = value.split_once('|') else {
        return false;
    };
    !pid.is_empty()
        && !ticks.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && ticks.bytes().all(|byte| byte.is_ascii_digit())
}
