use super::{AppServerError, Duration, Value, Work, held, json, transport};

impl Work {
    pub(super) async fn release(mut self) -> Result<(), AppServerError> {
        if self.token.state == "AwaitUnload" {
            return self.confirm_unloaded().await;
        }
        let eligibility = self.prove_idle().await;
        let watermark = match eligibility {
            Ok(revision) => revision,
            Err(error) => {
                self.advance("Candidate", &error.to_string())?;
                return Err(error);
            }
        };
        self.advance("Dispatching", "exact unsubscribe permission committed")?;
        let (result, phase) = self
            .rpc(
                "thread/unsubscribe",
                json!({"threadId":self.token.thread_id}),
                Duration::from_secs(8),
                true,
                Some(watermark),
            )
            .await;
        match result {
            Ok(value)
                if matches!(
                    value.get("status").and_then(Value::as_str),
                    Some("unsubscribed" | "notSubscribed" | "notLoaded")
                ) =>
            {
                // Even ACK notLoaded is kept distinct from a subsequent fresh unload probe.
                self.advance("AwaitUnload", "unsubscribe acknowledged; unload unverified")?;
                Ok(())
            }
            other => {
                let error = other
                    .err()
                    .unwrap_or_else(|| held("unclassifiable unsubscribe reply"));
                let (state, detail) = if phase == transport::WritePhase::NotStarted {
                    ("Settled", "CancelledBeforeSend".to_owned())
                } else {
                    ("Unknown", error.to_string())
                };
                if let Err(recording) = self.advance(state, &detail) {
                    return Err(held(format!(
                        "{error}; recording failed: {recording}; durable dispatch hold retained"
                    )));
                }
                Err(error)
            }
        }
    }

    async fn prove_idle(&self) -> Result<u64, AppServerError> {
        let watermark = self.local_idle(None)?;
        if !self
            .admission
            .client
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .witnessed_idle_terminal(&self.token.thread_id, &self.token.turn_id)
        {
            return Err(held(
                "idle unverified: exact terminal not witnessed by this resident",
            ));
        }
        let (goal, _) = self
            .rpc(
                "thread/goal/get",
                json!({"threadId":self.token.thread_id}),
                Duration::from_secs(2),
                true,
                Some(watermark),
            )
            .await;
        let goal = goal?;
        if goal.get("goal").is_none() {
            return Err(held("idle unverified: missing explicit goal result"));
        }
        let goal = crate::goal::parse_thread_goal_status(&goal, &self.token.thread_id)
            .map_err(|e| held(e.to_string()))?;
        if !matches!(goal, None | Some(crate::goal::ThreadGoalStatus::Complete)) {
            return Err(held(
                "idle release deferred: Goal remains active, paused, blocked or limited",
            ));
        }
        let (thread, _) = self
            .rpc(
                "thread/read",
                json!({"threadId":self.token.thread_id,"includeTurns":true}),
                Duration::from_secs(2),
                true,
                Some(watermark),
            )
            .await;
        let thread = thread?;
        let latest = thread
            .pointer("/thread/turns")
            .and_then(Value::as_array)
            .and_then(|t| t.last());
        if thread.pointer("/thread/id").and_then(Value::as_str)
            != Some(self.token.thread_id.as_str())
            || thread
                .pointer("/thread/status/type")
                .and_then(Value::as_str)
                != Some("idle")
            || latest.and_then(|t| t.get("id")).and_then(Value::as_str)
                != Some(self.token.turn_id.as_str())
            || !matches!(
                latest.and_then(|t| t.get("status")).and_then(Value::as_str),
                Some("completed" | "failed" | "interrupted")
            )
        {
            return Err(held(
                "idle unverified: exact latest terminal turn is missing or thread not idle",
            ));
        }
        self.local_idle(Some(watermark))
    }

    async fn confirm_unloaded(&mut self) -> Result<(), AppServerError> {
        // thread/closed is intentionally not an authority: it has no intent revision.
        let (value, _) = self
            .rpc(
                "thread/read",
                json!({"threadId":self.token.thread_id,"includeTurns":false}),
                Duration::from_secs(2),
                false,
                None,
            )
            .await;
        let value = value?;
        if value.pointer("/thread/id").and_then(Value::as_str)
            != Some(self.token.thread_id.as_str())
        {
            return Err(held("unload read returned a different or missing thread"));
        }
        if value.pointer("/thread/status/type").and_then(Value::as_str) == Some("notLoaded") {
            self.advance("Settled", "UnloadedConfirmed")?;
        }
        Ok(())
    }
}
