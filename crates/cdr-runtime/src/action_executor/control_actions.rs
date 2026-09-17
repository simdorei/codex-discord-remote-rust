use super::{ActionError, ActionExecutor, ActionResult, immediate};
use crate::queue_runner::TurnBackend;
use cdr_app_server::goal::parse_thread_goal_status;
use cdr_app_server::requests::{get_goal, rate_limits, usage};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn status(
        &self,
        channel_id: u64,
        reference: Option<&str>,
    ) -> Result<ActionResult, ActionError> {
        let thread = self.resolve_thread(channel_id, reference)?;
        let thread_id = thread.id.clone();
        let mut lines = vec![
            format!("Codex thread status\nthread_id: {thread_id}"),
            format!("title: {}", thread.title),
            format!("cwd: {}", thread.cwd),
            format!("마지막 저장 model: {}", thread.model),
            format!("마지막 저장 reasoning: {}", thread.reasoning_effort),
            "현재 실행 설정: 미확인 (저장값과 다를 수 있음)".into(),
            format!(
                "tokens_used: {} (누적 사용량)",
                thread
                    .tokens_used
                    .filter(|v| *v >= 0)
                    .map_or_else(|| "미확인".into(), |v| v.to_string())
            ),
        ];
        if let Some(server) = &self.server {
            let generation = server.generation();
            let state =
                super::list_view::states(Some(server), std::slice::from_ref(&thread), 1).await;
            lines.push(format!(
                "state: {}",
                state.get(&thread_id).map_or("미확인", String::as_str)
            ));
            let goal = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                server.execute(get_goal(&thread_id), Some(generation)),
            )
            .await;
            let goal = match goal {
                Ok(Ok(value))
                    if generation == server.generation() && value.get("goal").is_some() =>
                {
                    match parse_thread_goal_status(&value, &thread_id) {
                        Ok(Some(status)) => format!("goal: {status:?} (서버 조회)"),
                        Ok(None) => "goal: 등록된 목표 없음 (서버 조회)".into(),
                        Err(error) => format!("goal 조회 실패: {error}"),
                    }
                }
                Ok(Ok(_)) => "goal 조회 실패: 서버 세대 변경 또는 goal 응답 필드 누락".into(),
                Ok(Err(error)) => format!("goal 조회 실패: {error}"),
                Err(_) => "goal 조회 실패: 전체 조회 시간 3초 초과; 요청 취소".into(),
            };
            lines.push(goal);
        } else {
            lines.push("현재 실행 상태 미확인: app-server 연결 없음".into());
            lines.push("goal 미확인: app-server 연결 없음".into());
        }
        match crate::context_view::render_status(thread).await {
            Ok(context) => lines.push(context),
            Err(error) => lines.push(format!("최근 대화 조회 실패: {error}")),
        }
        if reference.is_none() && self.target(channel_id)?.0 != thread_id {
            return Err(ActionError::Invalid(
                "status target changed during read".into(),
            ));
        }
        Ok(immediate(lines.join("\n")))
    }

    pub(super) async fn usage(&self, days: u32) -> Result<ActionResult, ActionError> {
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let rates = server
            .execute(rate_limits(), Some(server.generation()))
            .await?;
        let usage_value = server.execute(usage(), Some(server.generation())).await?;
        Ok(immediate(super::usage_format::format_usage(
            days,
            &rates,
            &usage_value,
            chrono::Utc::now().date_naive(),
        )))
    }
}

impl<B: TurnBackend> ActionExecutor<B> {
    /// Caller already owns `control_lock(target)`. No second lock or queue job.
    pub(crate) async fn prepare_async_reply_locked(
        &self,
        target: &str,
    ) -> Result<(), crate::queue_runner::BackendFailure> {
        self.queue.backend.prepare_turn(target).await
    }

    pub(crate) async fn note_async_usage_limit_locked(&self, target: &str) {
        let _ = self.queue.backend.note_usage_limit(target).await;
    }
}
