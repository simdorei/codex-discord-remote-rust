use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::evidence::{BROWSER_EVIDENCE_PROTOCOL, BROWSER_PROBE_SHA256, common};

const BLOCK_REASON: &str = "Unsupported Chrome unavailability claim blocked. Run the trusted same-turn Chrome evidence probe, or say exactly: Chrome bootstrap was not verified.";

pub(super) fn check(payload: &Value, data: &Path) -> Option<Value> {
    let message = payload["last_assistant_message"].as_str()?;
    if !claims_unavailable(message) {
        return None;
    }
    let authorized = payload["session_id"]
        .as_str()
        .zip(payload["turn_id"].as_str())
        .is_some_and(|(session, turn)| {
            let path = data
                .join("browser-evidence")
                .join(format!("{}.json", common::receipt_key(session, turn)));
            let Some(receipt) = common::read_json_object(&path) else {
                return false;
            };
            receipt.get("protocol").and_then(Value::as_str) == Some(BROWSER_EVIDENCE_PROTOCOL)
                && receipt.get("session_id").and_then(Value::as_str) == Some(session)
                && receipt.get("turn_id").and_then(Value::as_str) == Some(turn)
                && receipt.get("browser_type").and_then(Value::as_str) == Some("chrome")
                && receipt.get("status").and_then(Value::as_str) == Some("unavailable")
                && receipt
                    .get("can_report_unavailable")
                    .and_then(Value::as_bool)
                    == Some(true)
                && receipt.get("probe_sha256").and_then(Value::as_str) == Some(BROWSER_PROBE_SHA256)
        });
    (!authorized).then(|| json!({"decision":"block","reason":BLOCK_REASON}))
}

fn claims_unavailable(message: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    if message
        .trim()
        .trim_end_matches('.')
        .eq_ignore_ascii_case("chrome bootstrap was not verified")
    {
        return false;
    }
    PATTERN.get_or_init(|| Regex::new(r"(?is)(?:\bgoogle\s+chrome\b|\bchrome\b).{0,24}(?:\bunavailable\b|\bnot\s+available\b|\bcannot\s+be\s+used\b|\bcan['’]?t\s+be\s+used\b)|(?:구글\s*)?크롬.{0,30}(?:사용\s*불가|사용할\s*수\s*없|연결\s*불가|작동하지\s*않|안\s*(?:돼|됨)|불가능)").expect("static claims regex")).is_match(message)
}
