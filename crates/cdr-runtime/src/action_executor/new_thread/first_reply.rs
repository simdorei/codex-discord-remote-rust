use super::{ActionError, ActionExecutor, ActionResult, StoredIngress, TurnBackend};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn admit_new_prompt(
        &self,
        request: cdr_store::prompt_intake::NewPromptIntake<'_>,
        ingress: &StoredIngress,
        generation: i64,
    ) -> cdr_store::Result<cdr_store::prompt_intake::PromptIntakeAdmission> {
        let text = format!(
            "In progress\nmessage: {}\n새 대화: <#{}>",
            super::super::queue_result::request_echo(
                cdr_store::ingress::new_command_prompt(ingress).ok_or_else(|| {
                    cdr_store::StoreError::Integrity(
                        "new acknowledgement has no original prompt".into(),
                    )
                })?
            ),
            request.channel_id
        );
        let seed = self
            .mirror_sync
            .get()
            .map(|_| cdr_store::new_reply::NewReplySeed {
                state_db: &self.state_db,
                acknowledgement: &text,
            });
        cdr_store::prompt_intake::admit_prompt_intake_with_ingress_and_reply(
            &self.mirror_db,
            request,
            &ingress.ingress_id,
            generation,
            seed,
        )
    }

    pub(super) fn new_first_reply(
        &self,
        ingress: &StoredIngress,
        result: ActionResult,
    ) -> Result<ActionResult, ActionError> {
        if self.mirror_sync.get().is_none() {
            return Ok(result);
        }
        let record = cdr_store::new_reply::get_by_ingress(&self.mirror_db,&ingress.ingress_id)?
            .ok_or_else(||ActionError::Invalid("new request has no durable first-reply intent; existing execution is preserved for review".into()))?;
        cdr_store::new_reply::validate_current(&self.mirror_db, &record.identity.job_id)?;
        if record.turn_id.is_none() {
            return Err(ActionError::Invalid(format!(
                "new first turn acceptance is not confirmed; request remains saved without replay: {}",
                result.text
            )));
        }
        self.notify_delivery_ready();
        Ok(ActionResult {
            text: record.identity.acknowledgement,
            waits_for_final: true,
            ui: None,
        })
    }
}
