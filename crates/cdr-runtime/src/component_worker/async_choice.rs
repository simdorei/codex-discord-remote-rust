use super::{ComponentWorkerError, ConfirmationPlan};
use crate::{
    action_executor::ActionExecutor,
    discord_dispatch::{InboundInteractionWork, InteractionProcessingMode},
    queue_runner::TurnBackend,
};
use cdr_app_server::{
    AppServerError, ResidentAppServer,
    requests::{AppRequest, start_turn, steer_turn},
};
use cdr_store::async_question::{self as store, DispatchMode, Question};
use serde_json::json;
use std::time::Duration;

pub(super) async fn handle<B: TurnBackend>(
    work: &InboundInteractionWork,
    id: &str,
    option: usize,
    executor: &ActionExecutor<B>,
    server: &ResidentAppServer,
) -> Result<ConfirmationPlan, ComponentWorkerError> {
    let db = executor.mirror_db();
    let q = store::get(db, id)?;
    let _lock = executor
        .control_lock(&q.thread_id)
        .await
        .map_err(|e| invalid(&e.to_string()))?;
    let q = store::get(db, id)?;
    let message = work
        .source_message_id
        .ok_or(ComponentWorkerError::MissingSourceMessage)?
        .to_string();
    if u64::try_from(q.channel_id).ok() != Some(work.channel_id.get())
        || u64::try_from(q.owner_user_id).ok() != Some(work.user_id.get())
        || q.message_id.as_deref() != Some(&message)
        || option >= q.body.options.len()
    {
        return Err(invalid(
            "질문의 사용자·방·메시지·선택지가 일치하지 않아 답변하지 않았습니다.",
        ));
    }
    store::require_current_mapping(db, &q)?;
    if q.state == "submitted" {
        if q.chosen != Some(option) {
            return Err(ComponentWorkerError::AlreadyHandled);
        }
        return Ok(confirmation(&q)); // Display recovery only, never another RPC.
    }
    if work.processing_mode == InteractionProcessingMode::ConfirmationOnly {
        return Err(ComponentWorkerError::ActionUnconfirmed);
    }
    if q.state != "open" {
        return Err(invalid(&format!(
            "질문 답변 상태: {}. 자동 재전송하지 않습니다. {}",
            q.state, q.error
        )));
    }
    let mode = preflight(server, &q).await?;
    let prompt = answer_prompt(&q, option)?;
    store::begin_dispatch(
        db,
        &store::Claim {
            id,
            runtime_id: server.instance_id(),
            generation: q.generation,
            channel: q.channel_id,
            actor: q.owner_user_id,
            message: &message,
            option,
            mode,
            prompt: &prompt,
            now: super::now()?,
        },
    )?;
    // Actual-send boundary: original turn, no successor, connection and mapping
    // are checked again while the same target lock is held. No deferred starts.
    let checked = async {
        store::require_current_mapping(db, &q)?;
        if preflight(server, &q).await? != mode {
            return Err(invalid(
                "질문의 원래 작업 상태가 변경되어 답변하지 않았습니다.",
            ));
        }
        store::validate_dispatch_guards(db, &q.thread_id)?;
        Ok::<_, ComponentWorkerError>(())
    }
    .await;
    if let Err(error) = checked {
        store::reject_definite(db, id, &error.to_string())?;
        return Err(error);
    }
    dispatch_claimed(db, server, &q, mode, &prompt).await?;
    executor.notify_delivery_ready();
    Ok(confirmation(&q))
}

async fn dispatch_claimed(
    db: &std::path::Path,
    server: &ResidentAppServer,
    q: &Question,
    mode: DispatchMode,
    prompt: &str,
) -> Result<(), ComponentWorkerError> {
    let id = &q.id;
    let request = match mode {
        DispatchMode::Steer => steer_turn(&q.thread_id, prompt, &q.turn_id),
        DispatchMode::Start => start_turn(&q.thread_id, prompt),
    };
    let generation =
        u64::try_from(q.generation).map_err(|_| invalid("invalid question generation"))?;
    let response = match server.execute(request, Some(generation)).await {
        Ok(response) => response,
        Err(error) => {
            if mode == DispatchMode::Start
                && let AppServerError::Remote { data, .. } = &error
                && cdr_app_server::is_usage_limit_error(data.as_ref())
            {
                store::reject_usage_limit(db, id, &error.to_string())?;
                return Err(error.into());
            }
            if matches!(
                error,
                AppServerError::Remote { .. }
                    | AppServerError::GenerationMismatch { .. }
                    | AppServerError::GenerationQuarantined { .. }
                    | AppServerError::DeadGenerationFence { .. }
            ) {
                store::reject_definite(db, id, &error.to_string())?;
            } else {
                store::record_error(db, id, &error.to_string())?;
            }
            return Err(error.into());
        }
    };
    let accepted = match mode {
        DispatchMode::Steer => response.get("turnId"),
        DispatchMode::Start => response.get("turn").and_then(|t| t.get("id")),
    }
    .and_then(serde_json::Value::as_str);
    let Some(accepted) = accepted.filter(|v| !v.is_empty()) else {
        store::record_error(
            db,
            id,
            "app-server response omitted accepted turn identity; answer outcome held",
        )?;
        return Err(ComponentWorkerError::ActionOutcomeIndeterminate(
            "accepted turn identity missing".into(),
        ));
    };
    store::confirm_dispatch(db, id, accepted)?;
    Ok(())
}

async fn preflight(
    server: &ResidentAppServer,
    q: &Question,
) -> Result<DispatchMode, ComponentWorkerError> {
    let snapshot = server.lifecycle_snapshot().await;
    if !snapshot.healthy
        || snapshot.quarantined
        || snapshot.restart_pending
        || server.instance_id() != q.runtime_id
        || i64::try_from(snapshot.generation).ok() != Some(q.generation)
    {
        return Err(invalid(
            "질문을 만든 기존 연결이 변경되었거나 준비되지 않았습니다. 답변하지 않았습니다.",
        ));
    }
    if let Some(active) = server.active_turn_id(&q.thread_id).await? {
        return if active == q.turn_id {
            Ok(DispatchMode::Steer)
        } else {
            Err(invalid(
                "새 작업이 이미 시작되어 이전 질문의 버튼은 만료되었습니다.",
            ))
        };
    }
    let result = server
        .execute(
            AppRequest {
                method: "thread/turns/list",
                params: json!({
                    "threadId":q.thread_id,"limit":1,"sortDirection":"desc","itemsView":"full"
                }),
                timeout: Duration::from_secs(8),
            },
            Some(snapshot.generation),
        )
        .await?;
    let turns = result
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| invalid("최신 작업 기록을 확인할 수 없어 답변하지 않았습니다."))?;
    if turns.len() != 1 || turns[0]["id"] != q.turn_id || turns[0]["status"] != "completed" {
        return Err(invalid(
            "질문의 원래 작업이 마지막 완료 작업인지 확인되지 않았습니다. 이전 질문을 새 작업에 전달하지 않습니다.",
        ));
    }
    let goal = server
        .execute(
            cdr_app_server::requests::get_goal(&q.thread_id),
            Some(snapshot.generation),
        )
        .await?;
    if cdr_app_server::goal::parse_thread_goal_status(&goal, &q.thread_id)
        .map_err(|e| invalid(&e.to_string()))?
        == Some(cdr_app_server::goal::ThreadGoalStatus::Active)
    {
        return Err(invalid(
            "목표 작업이 계속 실행 중입니다. 원래 작업이 활성 상태일 때 답하거나 새 메시지로 요청해 주세요.",
        ));
    }
    Ok(DispatchMode::Start)
}

fn answer_prompt(q: &Question, option: usize) -> Result<String, ComponentWorkerError> {
    let selected = q
        .body
        .options
        .get(option)
        .ok_or(ComponentWorkerError::InvalidComponent)?;
    let data = json!({"thread_id":q.thread_id,"original_turn_id":q.turn_id,"question_item_id":q.item_id,
        "question_index":q.body.index,"question_title":q.body.title,"selected_option_index":option,"selected_option":selected});
    Ok(format!(
        "The user answered exactly this earlier async question through Discord. Apply this selection only to this question; other questions remain unanswered.\n{data}"
    ))
}

fn confirmation(q: &Question) -> ConfirmationPlan {
    ConfirmationPlan {
        content: "선택한 답변을 원래 Codex 스레드에 전달했습니다.",
        domain: "async-question-confirmation-v1",
        logical_key: q.id.clone(),
    }
}

fn invalid(message: &str) -> ComponentWorkerError {
    ComponentWorkerError::AsyncQuestion(message.into())
}
