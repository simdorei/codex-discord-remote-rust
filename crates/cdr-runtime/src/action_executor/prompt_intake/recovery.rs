use cdr_store::prompt_intake::{
    PromptIntakeClaim, list_prompt_intakes, list_ready_prompt_intakes,
    prompt_intake_has_durable_owner, release_all_prompt_intake_claims,
    remove_prompt_intake_if_queued, renew_prompt_intake_claim_if_current,
};
use tokio::time::{Duration, MissedTickBehavior, interval};

use super::{INTAKE_CLAIM_SECONDS, unix_now};
use crate::action_executor::{ActionError, ActionExecutor, ActionResult};
use crate::queue_runner::TurnBackend;

const INTAKE_CLAIM_RENEW_MINUTES: u64 = 2;

pub(super) enum ClaimProcessingOutcome {
    Finished(Result<ActionResult, ActionError>),
    Lost,
}

impl<B: TurnBackend> ActionExecutor<B> {
    pub async fn recover_prompt_intakes_on_startup(&self) -> Result<usize, ActionError> {
        let released = release_all_prompt_intake_claims(&self.mirror_db)?;
        if released != 0 {
            eprintln!("rust_prompt_intake_startup_claims_released: {released}");
        }
        self.recover_prompt_intakes().await
    }

    pub async fn recover_prompt_intakes(&self) -> Result<usize, ActionError> {
        self.cleanup_already_queued_intakes()?;
        let now = unix_now()?;
        let ready = list_ready_prompt_intakes(&self.mirror_db, now)?;
        let mut recovered = 0;
        let mut first_recording_error = None;
        for intake in ready {
            let Some(claim) = self.try_claim_intake(&intake)? else {
                continue;
            };
            match self.process_claim_with_renewal(&claim).await {
                ClaimProcessingOutcome::Finished(Ok(_)) => {
                    self.require_atomic_promotion(&claim.intake.job_id)?;
                    recovered += 1;
                }
                ClaimProcessingOutcome::Finished(Err(error)) => {
                    if remove_prompt_intake_if_queued(&self.mirror_db, &claim.intake.job_id)? {
                        recovered += 1;
                        eprintln!(
                            "prompt_intake_recovered_with_queue_warning job_id={} error={error}",
                            claim.intake.job_id
                        );
                    } else if cdr_store::prompt_intake::get_prompt_intake(
                        &self.mirror_db,
                        &claim.intake.job_id,
                    )?
                    .is_none()
                    {
                        recovered += 1;
                    } else {
                        match self.backoff_claim(&claim, &error) {
                            Ok(()) => eprintln!(
                                "prompt_intake_recovery_deferred job_id={} error={error}",
                                claim.intake.job_id
                            ),
                            Err(recording) => {
                                eprintln!(
                                    "prompt_intake_recovery_recording_error job_id={} error={recording}",
                                    claim.intake.job_id
                                );
                                if first_recording_error.is_none() {
                                    first_recording_error = Some(recording);
                                }
                            }
                        }
                    }
                }
                ClaimProcessingOutcome::Lost => {
                    if remove_prompt_intake_if_queued(&self.mirror_db, &claim.intake.job_id)? {
                        recovered += 1;
                    } else {
                        eprintln!(
                            "prompt_intake_recovery_claim_lost job_id={}",
                            claim.intake.job_id
                        );
                    }
                }
            }
        }
        first_recording_error.map_or(Ok(recovered), Err)
    }

    pub(super) async fn process_claim_with_renewal(
        &self,
        claim: &PromptIntakeClaim,
    ) -> ClaimProcessingOutcome {
        let mut current_claim = match self.renew_claim(claim) {
            Ok(Some(current_claim)) => current_claim,
            Ok(None) => return ClaimProcessingOutcome::Lost,
            Err(error) => return ClaimProcessingOutcome::Finished(Err(error)),
        };
        let mut renewal = interval(Duration::from_mins(INTAKE_CLAIM_RENEW_MINUTES));
        renewal.set_missed_tick_behavior(MissedTickBehavior::Delay);
        renewal.tick().await;
        let job_id = current_claim.intake.job_id.clone();
        let processing_claim = current_claim.clone();
        let processing = self.process_claim(&processing_claim);
        tokio::pin!(processing);
        loop {
            tokio::select! {
                biased;
                _ = renewal.tick() => {
                    current_claim = match self.renew_claim(&current_claim) {
                        Ok(Some(current_claim)) => current_claim,
                        Ok(None) => {
                            match prompt_intake_has_durable_owner(&self.mirror_db, &job_id) {
                                Ok(true) => {
                                    return ClaimProcessingOutcome::Finished(processing.await);
                                }
                                Ok(false) => return ClaimProcessingOutcome::Lost,
                                Err(error) => {
                                    return ClaimProcessingOutcome::Finished(Err(error.into()));
                                }
                            }
                        }
                        Err(error) => return ClaimProcessingOutcome::Finished(Err(error)),
                    };
                }
                result = &mut processing => return ClaimProcessingOutcome::Finished(result),
            }
        }
    }

    fn renew_claim(
        &self,
        claim: &PromptIntakeClaim,
    ) -> Result<Option<PromptIntakeClaim>, ActionError> {
        let now = unix_now()?;
        let expires_at = (now + INTAKE_CLAIM_SECONDS).max(claim.intake.claim_expires_at + 1.0);
        Ok(renew_prompt_intake_claim_if_current(
            &self.mirror_db,
            claim,
            now,
            expires_at,
        )?)
    }

    fn cleanup_already_queued_intakes(&self) -> Result<(), ActionError> {
        for intake in list_prompt_intakes(&self.mirror_db)? {
            let _ = remove_prompt_intake_if_queued(&self.mirror_db, &intake.job_id)?;
        }
        Ok(())
    }
}
