use super::{
    AppServerTurnBackend, BackendFailure, TurnBackend, definite, validate_thread_identity,
};
use cdr_app_server::requests::{AppRequest, read_thread_with_timeout};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};

impl AppServerTurnBackend {
    pub(super) async fn read_async_resolution_history(
        &self,
        thread: &str,
        originals: &[String],
    ) -> Result<Option<Value>, BackendFailure> {
        if originals.is_empty() {
            return Ok(None);
        }
        if originals.len() > 128 {
            return Err(BackendFailure::definite(
                "too many historical original turns",
            ));
        }
        let generation = self.generation();
        let timeout = self.history_read_timeout.min(Duration::from_secs(2));
        let metadata = self
            .server
            .execute(
                read_thread_with_timeout(thread, false, timeout),
                Some(generation),
            )
            .await
            .map_err(|e| definite(&e))?;
        validate_thread_identity(&metadata, thread).map_err(BackendFailure::definite)?;
        let wanted = originals
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut found = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut cursors = BTreeSet::new();
        let mut cursor = Value::Null;
        let mut turns = Vec::new();
        let mut bytes = 0usize;
        for _ in 0..8 {
            let page=self.server.execute(AppRequest {
                method:"thread/turns/list",
                params:json!({"threadId":thread,"limit":16,"sortDirection":"desc","itemsView":"full","cursor":cursor}),
                timeout,
            },Some(generation)).await.map_err(|e|definite(&e))?;
            bytes = bytes.saturating_add(page.to_string().len());
            if bytes > 1_048_576
                || page
                    .get("truncated")
                    .is_some_and(|v| v != &Value::Bool(false))
            {
                return Err(BackendFailure::definite(
                    "historical page truncated or exceeds byte bound",
                ));
            }
            let data = page
                .get("data")
                .and_then(Value::as_array)
                .filter(|v| v.len() <= 16)
                .ok_or_else(|| {
                    BackendFailure::definite("historical page has no bounded turn array")
                })?;
            for turn in data {
                let id = turn
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 512)
                    .ok_or_else(|| BackendFailure::definite("historical turn identity missing"))?;
                if !seen.insert(id.to_owned()) {
                    return Err(BackendFailure::definite(
                        "duplicate historical turn identity",
                    ));
                }
                if wanted.contains(id) {
                    found.insert(id.to_owned());
                    turns.push(turn.clone());
                }
            }
            let next = match page.get("nextCursor") {
                None | Some(Value::Null) => Value::Null,
                Some(Value::String(s)) if !s.is_empty() && s.len() <= 2048 => {
                    Value::String(s.clone())
                }
                _ => return Err(BackendFailure::definite("invalid historical cursor")),
            };
            if next.is_null() || wanted.iter().all(|id| found.contains(*id)) {
                if self.generation() != generation {
                    return Err(BackendFailure::definite("historical connection changed"));
                }
                return Ok(Some(
                    json!({"threadId":thread,"turns":turns,"history_exhausted":next.is_null()}),
                ));
            }
            if !cursors.insert(next.to_string()) {
                return Err(BackendFailure::definite("historical cursor repeated"));
            }
            cursor = next;
        }
        Err(BackendFailure::definite(
            "historical read exceeded eight pages; absence is not proof",
        ))
    }
}
