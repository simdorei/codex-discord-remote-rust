//! Discover the complete server-side archive scope before the mutating request.
use super::ActionError;
use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};

pub(super) async fn descendants(
    server: &ResidentAppServer,
    root: &str,
    generation: u64,
) -> Result<BTreeSet<String>, ActionError> {
    let mut ids = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..=10 {
        let value = server.execute(AppRequest {
            method: "thread/list",
            params: json!({"ancestorThreadId":root,"archived":false,"limit":100,"cursor":cursor,
                "sourceKinds":["cli","vscode","exec","appServer","subAgent","subAgentReview","subAgentCompact","subAgentThreadSpawn","subAgentOther","unknown"]}),
            timeout: Duration::from_secs(8),
        }, Some(generation)).await?;
        let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
            ActionError::Invalid("archive descendant inventory is missing or invalid".into())
        })?;
        for thread in data {
            let id = thread
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.trim() == *id)
                .ok_or_else(|| {
                    ActionError::Invalid("archive descendant has no exact identity".into())
                })?;
            if id == root || !ids.insert(id.to_owned()) || ids.len() > 100 {
                return Err(ActionError::Invalid("archive scope is repeated, inconsistent, or exceeds 100 descendants; no archive was sent".into()));
            }
        }
        match value.get("nextCursor") {
            Some(Value::Null) => return Ok(ids),
            Some(Value::String(next)) if !next.is_empty() && cursors.insert(next.clone()) => {
                cursor = Some(next.clone());
            }
            _ => {
                return Err(ActionError::Invalid(
                    "archive descendant pagination is missing or repeated; no archive was sent"
                        .into(),
                ));
            }
        }
    }
    Err(ActionError::Invalid(
        "archive descendant pagination exceeded its bound; no archive was sent".into(),
    ))
}
