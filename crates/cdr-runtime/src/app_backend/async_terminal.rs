use super::{
    AppServerTurnBackend, BackendFailure, TurnBackend, definite, validate_thread_identity,
};
use cdr_app_server::requests::{AppRequest, read_thread_with_timeout};
use serde_json::{Value, json};
use std::time::Duration;

impl AppServerTurnBackend {
    pub(super) async fn read_async_terminal_history(
        &self,
        thread: &str,
        owners: &[String],
    ) -> Result<Option<Value>, BackendFailure> {
        let generation = self.generation();
        let Some(mut history) = self.read_async_resolution_history(thread, owners).await? else {
            return Ok(None);
        };
        let timeout = self.history_read_timeout.min(Duration::from_secs(2));
        let goal = self
            .server
            .execute(
                AppRequest {
                    method: "thread/goal/get",
                    params: json!({"threadId":thread}),
                    timeout,
                },
                Some(generation),
            )
            .await
            .map_err(|e| definite(&e))?;
        let metadata = self
            .server
            .execute(
                read_thread_with_timeout(thread, false, timeout),
                Some(generation),
            )
            .await
            .map_err(|e| definite(&e))?;
        validate_thread_identity(&metadata, thread).map_err(BackendFailure::definite)?;
        if self.generation() != generation {
            return Err(BackendFailure::definite(
                "historical terminal connection changed",
            ));
        }
        history["goal_observation"] = goal;
        history["thread_observation"] = metadata;
        if history.to_string().len() > 1_048_576 {
            return Err(BackendFailure::definite(
                "historical terminal observation exceeds byte bound",
            ));
        }
        Ok(Some(history))
    }
}
