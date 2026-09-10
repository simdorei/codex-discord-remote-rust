use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PRO_SKILL_NAME: &str = "ask-chatgpt-pro";
pub const CHROME_MENTION_NAME: &str = "Chrome";
pub const CHROME_PLUGIN_URI: &str = "plugin://chrome@openai-bundled";
pub const PRO_SKILL_CALL: &str = "$ask-chatgpt-pro [@Chrome](plugin://chrome@openai-bundled)";
pub const PRO_REVIEW_MARKER: &str = "<pro-review>";
pub const PRODUCTION_CONNECTOR_NAME: &str = "Simdorei Local Project Oauth";
pub const PRODUCTION_CONNECTOR_RESOURCE: &str = "https://simdorei.duckdns.org/mcp";
const PRO_CONVERSATION_SCOPE_LENGTH: usize = 24;
const DEVICE_INSTRUCTION: &str = concat!(
    "\nUse only the connector named in this tag and select it explicitly.\n",
    "Use PC mode by default.\n",
    "Call list_devices, verify that device_id is online, then call select_device\n",
    "exactly once with the device_id, working_directory, and connector resource\n",
    "from this tag. The working directory identifies the project for this ticket.\n",
    "Read a file before updating it and pass its SHA-256 when writing an existing file.\n",
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceTicket {
    pub device_id: String,
    pub working_directory: std::path::PathBuf,
}

#[must_use]
pub fn rewrite_pro_prompt(prompt: &str) -> Option<String> {
    let (command, request) = split_first_word(prompt)?;
    if !command.eq_ignore_ascii_case("!pro") {
        return None;
    }
    if let Some(request) = request {
        let (first, review_request) = split_first_word(request).expect("nonempty request");
        if first.eq_ignore_ascii_case("review") {
            return Some(match review_request {
                Some(value) => format!("{PRO_SKILL_CALL} {PRO_REVIEW_MARKER}\n{value}"),
                None => format!("{PRO_SKILL_CALL} {PRO_REVIEW_MARKER}"),
            });
        }
        return Some(format!("{PRO_SKILL_CALL} {request}").trim_end().to_owned());
    }
    Some(PRO_SKILL_CALL.to_owned())
}

#[must_use]
pub fn is_pro_command(prompt: &str) -> bool {
    rewrite_pro_prompt(prompt).is_some()
}

#[must_use]
pub fn pro_conversation_scope(thread_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(thread_id.as_bytes()));
    format!("codex-pro-{}", &digest[..PRO_CONVERSATION_SCOPE_LENGTH])
}

#[must_use]
pub fn format_local_device_prompt(
    rewritten_prompt: &str,
    target_thread_id: &str,
    ticket: &DeviceTicket,
) -> String {
    let connector = escape_xml_attribute(PRODUCTION_CONNECTOR_NAME);
    let resource = escape_xml_attribute(PRODUCTION_CONNECTOR_RESOURCE);
    let device_id = escape_xml_attribute(&ticket.device_id);
    let working_directory = escape_xml_attribute(&ticket.working_directory.to_string_lossy());
    let scope = escape_xml_attribute(&pro_conversation_scope(target_thread_id));
    format!(
        "{rewritten_prompt}\n<local-device-mcp connector=\"{connector}\" resource=\"{resource}\" device_id=\"{device_id}\" working_directory=\"{working_directory}\" conversation_scope=\"{scope}\">{DEVICE_INSTRUCTION}</local-device-mcp>"
    )
}

fn split_first_word(value: &str) -> Option<(&str, Option<&str>)> {
    let value = value.trim_start();
    if value.is_empty() {
        return None;
    }
    let end = value.find(char::is_whitespace).unwrap_or(value.len());
    let first = &value[..end];
    let remainder = value[end..].trim_start();
    Some((first, (!remainder.is_empty()).then_some(remainder)))
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[must_use]
pub fn is_pro_skill_prompt(prompt: &str) -> bool {
    prompt
        .strip_prefix(PRO_SKILL_CALL)
        .is_some_and(|remainder| {
            remainder.is_empty() || remainder.chars().next().is_some_and(char::is_whitespace)
        })
}

#[must_use]
pub fn build_turn_input(prompt: &str, skill_path: &Path) -> Vec<Value> {
    let mut input = vec![serde_json::json!({
        "type": "text",
        "text": prompt,
        "text_elements": [],
    })];
    if is_pro_skill_prompt(prompt) {
        input.extend([
            serde_json::json!({
                "type": "skill",
                "name": PRO_SKILL_NAME,
                "path": skill_path.to_string_lossy(),
            }),
            serde_json::json!({
                "type": "mention",
                "name": CHROME_MENTION_NAME,
                "path": CHROME_PLUGIN_URI,
            }),
        ]);
    }
    input
}
