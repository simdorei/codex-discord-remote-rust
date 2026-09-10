use std::time::SystemTime;

use cdr_discord::gateway::ingress::{MessageGapFence, MessageGapReceiver};

use super::{HistoryAdmittedMessage, HistoryMessageCandidate};
use crate::history_poll::HistoryClaimOutcome;
use crate::message_worker::{
    MessageAdmissionError, admit_message_candidate_at, discard_message_candidate_at,
};

pub(super) struct DiscordHistoryDiscardFence<'a> {
    gaps: &'a MessageGapReceiver,
    fence: MessageGapFence,
}

impl<'a> DiscordHistoryDiscardFence<'a> {
    pub(super) const fn new(gaps: &'a MessageGapReceiver, fence: MessageGapFence) -> Self {
        Self { gaps, fence }
    }

    pub(super) fn claim(
        &self,
        item: HistoryMessageCandidate,
        observed_at: SystemTime,
    ) -> Result<HistoryClaimOutcome<HistoryAdmittedMessage>, MessageAdmissionError> {
        self.gaps
            .with_current_fence(&self.fence, || discard_candidate_at(item, observed_at))?
    }
}

pub(super) fn claim_candidate_at(
    item: HistoryMessageCandidate,
    observed_at: SystemTime,
) -> Result<HistoryClaimOutcome<HistoryAdmittedMessage>, MessageAdmissionError> {
    Ok(
        match admit_message_candidate_at(item.candidate, observed_at)? {
            Some(admitted) => HistoryClaimOutcome::Won(HistoryAdmittedMessage::Process {
                admitted,
                report_target: item.report_target,
            }),
            None => HistoryClaimOutcome::Lost,
        },
    )
}

pub(super) fn discard_candidate_at(
    item: HistoryMessageCandidate,
    observed_at: SystemTime,
) -> Result<HistoryClaimOutcome<HistoryAdmittedMessage>, MessageAdmissionError> {
    Ok(
        if discard_message_candidate_at(item.candidate, observed_at)? {
            HistoryClaimOutcome::Won(HistoryAdmittedMessage::Discarded)
        } else {
            HistoryClaimOutcome::Lost
        },
    )
}
