use super::recovery_custody::RecoveryGuard;
use super::{ActionError, ActionExecutor, ActionResult, immediate};
use crate::{command_plan::CommandAction, queue_runner::TurnBackend};
use cdr_app_server::{AppServerError, ResidentAppServer};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::OwnedMutexGuard;

const WAIT: Duration = Duration::from_secs(12);
const INIT: &str =
    "if (!globalThis.sky) { const { sky } = await import(\"@oai/sky\"); globalThis.sky = sky; }";
const PROBE: &str =
    "nodeRepl.write(JSON.stringify({cdrRepair: 'ready', apps: (await sky.list_apps()).length}));";

// An RPC timeout does not mean a late reset stopped running. Keep this target
// locked until the old backend has been replaced; !recover does not need it.
struct RepairLease {
    lock: Option<OwnedMutexGuard<()>>,
    server: Arc<ResidentAppServer>,
    generation: u64,
    uncertain: bool,
    receipt: PathBuf,
    operation_id: String,
    stage: &'static str,
    guard: RecoveryGuard,
}

impl RepairLease {
    fn record(&self, phase: &str, thread: &str) -> Result<(), ActionError> {
        let value = json!({"phase":phase,"stage":self.stage,"operation_id":self.operation_id,
            "thread_id":thread,"generation":self.generation,
            "server_instance":self.server.instance_id(),"app_restarted":false});
        std::fs::write(&self.receipt, value.to_string())?;
        Ok(())
    }

    async fn call(
        &mut self,
        thread: &str,
        stage: &'static str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, ActionError> {
        self.stage = stage;
        self.record("tool_call_pending", thread)?;
        self.uncertain = true;
        let rejected = Arc::new(AtomicBool::new(false));
        let result = self
            .server
            .request_for_tool_repair_checked(
                "mcpServer/tool/call",
                json!({"threadId":thread,"server":"node_repl","tool":tool,"arguments":arguments}),
                WAIT,
                self.generation,
                self.guard.rpc_check(Some(Arc::clone(&rejected))),
            )
            .await;
        // A returned tool result or JSON-RPC error is a definite response. A
        // transport error, cancellation or deadline cannot prove the reset ended.
        let refused = rejected.load(Ordering::Acquire);
        if refused || result.is_ok() || matches!(result, Err(AppServerError::Remote { .. })) {
            self.uncertain = false;
        }
        self.record(
            if refused {
                "admission_refused_before_send"
            } else if self.uncertain {
                "outcome_unknown"
            } else if result.is_err() {
                "failed"
            } else {
                "tool_call_returned"
            },
            thread,
        )?;
        let value = result.map_err(|e| ActionError::Invalid(format!(
            "도구 복구 단계 {stage} 응답을 확인하지 못했습니다: {e}. 결과가 불명확하면 이 채팅의 새 실행을 보류합니다. !recover로 요청 취소·앱 재시작을 할 수 있습니다.")))?;
        if value.get("isError").is_some_and(|flag| !flag.is_boolean())
            || !value.get("content").is_some_and(Value::is_array)
        {
            self.record("failed", thread)?;
            return Err(ActionError::Invalid(format!(
                "도구 복구 단계 {stage} 응답 형식이 올바르지 않습니다. 앱은 재시작하지 않았습니다."
            )));
        }
        if value.get("isError").and_then(Value::as_bool) == Some(true) {
            self.record("failed", thread)?;
            let text = content_text(&value);
            let next = if text.contains("native pipe") {
                " 앱 연결 복구는 !recover를 사용하세요."
            } else {
                ""
            };
            return Err(ActionError::Invalid(format!(
                "도구 복구 단계 {stage} ({tool}) 실패: {}. 앱은 재시작하지 않았습니다.{next}",
                text.chars().take(1200).collect::<String>()
            )));
        }
        Ok(value)
    }
}

impl Drop for RepairLease {
    fn drop(&mut self) {
        if self.uncertain {
            let lock = self.lock.take();
            let server = Arc::clone(&self.server);
            let generation = self.generation;
            tokio::spawn(async move {
                let _lock = lock;
                while server.generation() == generation {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        }
    }
}

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn repair_tools(
        &self,
        channel: u64,
        reference: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        let guard = self.freeze_recovery(
            channel,
            &CommandAction::Repair {
                reference: reference.map(str::to_owned),
            },
        )?;
        self.repair_tools_bound(guard).await
    }

    pub(super) async fn repair_tools_bound(
        &self,
        guard: RecoveryGuard,
    ) -> Result<ActionResult, ActionError> {
        tokio::time::timeout(Duration::from_secs(25), self.repair_tools_guarded(&guard)).await
            .map_err(|_| ActionError::Invalid("도구 복구 제한시간 25초를 넘겼습니다. 결과가 불명확한 채팅은 보류하며, !recover로 요청 취소·앱 재시작을 할 수 있습니다.".into()))?
    }

    async fn repair_tools_guarded(
        &self,
        guard: &RecoveryGuard,
    ) -> Result<ActionResult, ActionError> {
        let thread = guard.target();
        let lock = tokio::time::timeout(Duration::from_secs(1), self.control_lock(thread))
            .await
            .map_err(|_| busy())??;
        guard.check()?;
        if !cdr_store::queue::list_filtered(&self.mirror_db, Some(thread), None)?.is_empty()
            || cdr_store::prompt_intake::list_prompt_intakes(&self.mirror_db)?
                .iter()
                .any(|v| v.target_thread_id == thread)
        {
            return Err(busy());
        }
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let generation = server.generation();
        let health = server.lifecycle_snapshot().await;
        if !health.healthy || health.quarantined || health.restart_pending {
            return Err(ActionError::Invalid("앱서버 연결이 비정상이라 채팅별 도구 초기화를 보낼 수 없습니다. !recover로 요청 취소·앱 재시작을 실행하세요.".into()));
        }
        let status = server
            .request_for_tool_repair_checked(
                "thread/read",
                json!({"threadId":thread,"includeTurns":false}),
                WAIT,
                generation,
                guard.rpc_check(None),
            )
            .await?;
        if status
            .pointer("/thread/status/type")
            .and_then(Value::as_str)
            != Some("idle")
            || status.pointer("/thread/id").and_then(Value::as_str) != Some(thread)
            || server.active_turn_id(thread).await?.is_some()
        {
            return Err(busy());
        }
        let mut cursor = Value::Null;
        let mut available = false;
        for _ in 0..4 {
            let inventory = server.request_for_tool_repair_checked("mcpServerStatus/list",
                json!({"threadId":thread,"detail":"toolsAndAuthOnly","limit":100,"cursor":cursor}), WAIT, generation, guard.rpc_check(None)).await?;
            available = node_tools_available(&inventory);
            cursor = inventory.get("nextCursor").cloned().unwrap_or(Value::Null);
            if available || cursor.is_null() {
                break;
            }
        }
        if !available {
            return Err(ActionError::Invalid("이 채팅에서 node_repl의 js/js_reset 연결을 확인하지 못했습니다. 지원되지 않는 도구는 초기화하지 않았습니다.".into()));
        }
        guard.check()?;
        let receipt_dir = self
            .mirror_db
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("maintenance_backups")
            .join("tool-repair");
        std::fs::create_dir_all(&receipt_dir)?;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let mut lease = RepairLease {
            lock: Some(lock),
            server: Arc::clone(server),
            generation,
            uncertain: false,
            receipt: receipt_dir.join(format!("{operation_id}.json")),
            operation_id,
            stage: "preflight",
            guard: guard.clone(),
        };
        let reset = lease.call(thread, "reset", "js_reset", json!({})).await?;
        if !content_text(&reset).contains("js kernel reset") {
            lease.record("failed", thread)?;
            return Err(ActionError::Invalid("도구 복구 단계 reset 응답 형식이 달라 완료를 확인하지 못했습니다. 앱은 재시작하지 않았습니다.".into()));
        }
        lease.call(thread, "initialize", "js", json!({"code":INIT,"timeout_ms":8000,"title":"Initialize Computer Use after repair"})).await?;
        let probe = lease.call(thread, "probe", "js", json!({"code":PROBE,"timeout_ms":8000,"title":"Verify repaired Computer Use connection"})).await?;
        if !probe_ready(&probe) || server.generation() != generation {
            lease.record("failed", thread)?;
            return Err(ActionError::Invalid("도구 복구 단계 probe 실패: JS 세션은 초기화했지만 Computer Use 연결을 확인하지 못했습니다. 앱은 재시작하지 않았습니다.".into()));
        }
        guard.check()?;
        lease.record("verified", thread)?;
        Ok(immediate(format!(
            "도구 복구 완료: {thread}\n이 채팅의 JS 세션을 초기화하고 Computer Use 연결을 확인했습니다. 앱 재시작·요청 취소는 하지 않았습니다. 기존 JS 변수와 도구 핸들은 다시 만들어야 합니다."
        )))
    }
}

fn busy() -> ActionError {
    ActionError::Invalid("이 채팅에 진행 중·대기 중 요청이 있거나 유휴 상태를 확인할 수 없어 도구를 초기화하지 않았습니다. 요청 취소와 앱 재시작은 !recover를 사용하세요.".into())
}

fn node_tools_available(inventory: &Value) -> bool {
    inventory
        .get("data")
        .and_then(Value::as_array)
        .is_some_and(|servers| {
            servers.iter().any(|server| {
                server["name"] == "node_repl"
                    && server["runtimeStatus"] == "connected"
                    && server.get("toolsError").is_none_or(Value::is_null)
                    && server.pointer("/tools/js/name").and_then(Value::as_str) == Some("js")
                    && server
                        .pointer("/tools/js_reset/name")
                        .and_then(Value::as_str)
                        == Some("js_reset")
            })
        })
}

fn content_text(value: &Value) -> String {
    value
        .get("content")
        .and_then(Value::as_array)
        .map_or_else(String::new, |content| {
            content
                .iter()
                .filter(|item| item["type"] == "text")
                .filter_map(|item| item["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn probe_ready(value: &Value) -> bool {
    content_text(value)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|line| line["cdrRepair"] == "ready" && line["apps"].as_u64().is_some())
}

#[cfg(test)]
#[path = "repair_action_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "repair_contract_patch10_tests.rs"]
mod patch10_tests;
