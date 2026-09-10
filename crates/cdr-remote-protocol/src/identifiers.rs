use std::sync::OnceLock;

use cdr_core::{Validate, ValidationError, ValidationResult};
use regex::Regex;
use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident, $pattern:literal, $field:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl Validate for $name {
            fn validate(&self) -> ValidationResult {
                static PATTERN: OnceLock<Regex> = OnceLock::new();
                let pattern = PATTERN.get_or_init(|| Regex::new($pattern).expect("valid regex"));
                if pattern.is_match(&self.0) {
                    Ok(())
                } else {
                    Err(ValidationError::new($field, "invalid identifier"))
                }
            }
        }
    };
}

string_id!(TerminalId, r"^term_[a-f0-9]{16}$", "terminal_id");
string_id!(TerminalReceiptId, r"^tr_[a-f0-9]{16}$", "receipt_id");
string_id!(
    TerminalWindowId,
    r"^termwin_[a-f0-9]{16}$",
    "terminal_window_id"
);
string_id!(
    TerminalWindowObservationId,
    r"^twobs_[a-f0-9]{16}$",
    "observation_id"
);
string_id!(
    TerminalWindowReceiptId,
    r"^twrcpt_[a-f0-9]{16}$",
    "receipt_id"
);

pub fn validate_sha256(field: &'static str, value: &str) -> ValidationResult {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(ValidationError::new(
            field,
            "must be 64 lowercase hexadecimal characters",
        ))
    }
}
