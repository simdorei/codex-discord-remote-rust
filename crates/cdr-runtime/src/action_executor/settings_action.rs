use super::model_catalog::reserve;
use super::{ActionError, ActionExecutor, ActionResult, immediate};
use crate::queue_runner::TurnBackend;
use cdr_app_server::{
    ResidentAppServer, extract_thread_id,
    requests::{
        ServiceTierUpdate, ThreadSettingsUpdate, list_models, rate_limits,
        resume_thread_with_timeout,
    },
};
use std::time::Duration;
use tokio::time::Instant;
pub(crate) mod snapshot;
pub(crate) mod verification;

struct Change<'a> {
    model: Option<&'a str>,
    effort: Option<&'a str>,
    speed: Option<&'a str>,
    binding: Option<&'a crate::settings_binding::SettingsBinding>,
}

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn settings_command(
        &self,
        channel: u64,
        action: crate::command_plan::CommandAction,
    ) -> Result<ActionResult, ActionError> {
        use crate::command_plan::CommandAction;
        match action {
            CommandAction::Settings {
                reference,
                model,
                effort,
                speed,
            } => {
                self.settings(
                    channel,
                    reference.as_deref(),
                    model.as_deref(),
                    effort.as_deref(),
                    speed.as_deref(),
                    None,
                )
                .await
            }
            CommandAction::AutoReserve { .. } => {
                Ok(immediate(crate::command_plan::AUTO_RESERVE_REMOVED))
            }
            _ => Err(ActionError::Invalid("not a settings command".into())),
        }
    }

    pub(super) async fn settings(
        &self,
        channel: u64,
        reference: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        speed: Option<&str>,
        binding: Option<&crate::settings_binding::SettingsBinding>,
    ) -> Result<ActionResult, ActionError> {
        let thread = if let Some(binding) = binding {
            self.settings_resolver().validate(binding, channel)?;
            self.resolve_reference(&binding.target, false)?
        } else {
            self.resolve_thread(channel, reference)?
        };
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let deadline = Instant::now() + self.app_server_resume_timeout.min(Duration::from_secs(20));
        if model.is_none() && effort.is_none() && speed.is_none() {
            let generation = server.generation();
            ready(server, generation).await?;
            let text = if let Some((_, value)) =
                server.observed_thread_settings(&thread.id, generation)?
            {
                snapshot::Settings::parse(&value)?.display(&thread.id, "마지막 서버 확인값")
            } else {
                format!(
                    "대화 설정 · 현재 실행값 미확인\nthread: {}\n마지막 저장 모델: {}\n마지막 저장 추론: {}\n속도: 확인된 기록 없음",
                    thread.id, thread.model, thread.reasoning_effort
                )
            };
            ready(server, generation).await?;
            if reference.is_none() && self.target(channel)?.0 != thread.id {
                return Err(ActionError::Invalid(
                    "settings query target changed; no current-room settings confirmed".into(),
                ));
            }
            return Ok(immediate(text));
        }
        let _guard = tokio::time::timeout_at(deadline, self.control_lock(&thread.id))
            .await
            .map_err(|_| {
                ActionError::Invalid("settings control wait timed out; no update was sent".into())
            })??;
        let generation = server.generation();
        let operation = self.apply_settings(
            channel,
            reference,
            &thread.id,
            generation,
            Change {
                model,
                effort,
                speed,
                binding,
            },
        );
        tokio::time::timeout_at(deadline,operation).await.map_err(|_|ActionError::Invalid("settings verification timed out; the update outcome is unverified, do not assume it was not applied".into()))?
    }

    async fn apply_settings(
        &self,
        channel: u64,
        reference: Option<&str>,
        thread: &str,
        generation: u64,
        change: Change<'_>,
    ) -> Result<ActionResult, ActionError> {
        let Change {
            model,
            effort,
            speed,
            binding,
        } = change;
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let mut service_tier = requested_tier(speed)?;
        ready(server, generation).await?;
        let mut catalog = server.execute(list_models(), Some(generation)).await?;
        let requested_reserve = model.is_some_and(reserve::requested);
        if requested_reserve {
            let rates = server.execute(rate_limits(), Some(generation)).await?;
            catalog = reserve::catalog(&catalog, &rates)?;
        }
        let model = model
            .map(|value| super::model_catalog::canonical_model(&catalog, value))
            .transpose()?;
        self.validate_settings_route(channel, reference, thread, binding)?;
        let resumed = server
            .execute(
                resume_thread_with_timeout(thread, Duration::from_secs(8)),
                Some(generation),
            )
            .await
            .map_err(|error| {
                super::operator_actions::original_owner_error("settings", thread, error)
            })?;
        if extract_thread_id(&resumed).as_deref() != Some(thread) {
            return Err(ActionError::Invalid(
                "settings resume returned a different or missing thread; no update was sent".into(),
            ));
        }
        let current = snapshot::Settings::from_resume(&resumed)?;
        let is_reserve = model.as_deref().unwrap_or(&current.model) == reserve::MODEL;
        let mut applied_effort = effort.map(str::to_owned);
        let mut stored_speed = speed;
        if is_reserve {
            if !requested_reserve {
                let rates = server.execute(rate_limits(), Some(generation)).await?;
                catalog = reserve::catalog(&catalog, &rates)?;
            }
            if speed == Some("fast") {
                return Err(ActionError::Invalid(
                    "Luna Reserve uses standard speed; no settings update was sent".into(),
                ));
            }
            applied_effort = Some(reserve::effort(
                &catalog,
                effort,
                current.effort.as_deref(),
            )?);
            // Select standard explicitly: the installed server can report "default"
            // after a null clear. Keep exact verification of the effective tier.
            service_tier = ServiceTierUpdate::Set("default".into());
            stored_speed = Some("standard");
        } else if let Some(effort) = effort {
            super::model_catalog::validate_effort(
                &catalog,
                model.as_deref().unwrap_or(&current.model),
                effort,
            )?;
        }
        let update = ThreadSettingsUpdate {
            model,
            effort: applied_effort,
            effort_clear: false,
            service_tier,
        };
        ready(server, generation).await?;
        self.validate_settings_route(channel, reference, thread, binding)?;
        let (applied, already_applied) =
            verification::apply_or_confirm(server, thread, generation, current, &update).await?;
        ready(server, generation).await?;
        self.validate_settings_route(channel, reference, thread, binding)?;
        self.bridge_state.remember_thread_settings(
            thread,
            update.model.as_deref(),
            update.effort.as_deref(),
            stored_speed,
        )?;
        let result = if already_applied {
            applied.display(thread, "이미 적용된 설정 확인 · 변경 요청을 보내지 않음")
        } else if is_reserve {
            applied.display(thread, "Luna Reserve 설정 변경 확인 · 다음 요청부터 적용 (이전 실패 요청은 재실행하지 않음)")
        } else if update.model.is_some() && effort.is_none() && speed.is_none() {
            format!("모델이 변경되었습니다: {}", applied.model)
        } else {
            applied.display(thread, "설정 변경 확인 · 다음 요청부터 적용")
        };
        Ok(immediate(result))
    }
}

pub(super) async fn ready(server: &ResidentAppServer, generation: u64) -> Result<(), ActionError> {
    let state = server.lifecycle_snapshot().await;
    if state.generation != generation
        || !state.healthy
        || state.quarantined
        || state.restart_pending
    {
        return Err(ActionError::Invalid(
            "settings connection changed or is unavailable; no verified success".into(),
        ));
    }
    Ok(())
}

fn requested_tier(speed: Option<&str>) -> Result<ServiceTierUpdate, ActionError> {
    match speed {
        Some("fast") => Ok(ServiceTierUpdate::Set("priority".into())),
        Some("standard") => Ok(ServiceTierUpdate::Clear),
        None => Ok(ServiceTierUpdate::Unchanged),
        Some(_) => Err(ActionError::Invalid(
            "unsupported speed; use standard or fast".into(),
        )),
    }
}
