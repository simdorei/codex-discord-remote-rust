use std::collections::HashSet;

use super::*;
use crate::history_poll::{HistoryPollItem, HistoryWatermark};

pub(super) const CHANNEL: u64 = 7;

pub(super) enum PayloadKind {
    Ignore,
    Candidate,
}

pub(super) struct Payload(u8, i64, PayloadKind, Box<str>);
pub(super) struct Work(u8, Box<str>);
pub(super) struct Admitted(Work);

#[derive(Default)]
pub(super) struct Script {
    payloads: Vec<Payload>,
    events: Vec<String>,
    pub(super) source_error: bool,
    pub(super) adaptation_error: Option<u8>,
    pub(super) claim_error: Option<u8>,
    pub(super) process_error: Option<u8>,
    pub(super) lost_claims: HashSet<u8>,
    claimed_ids: HashSet<u8>,
    pub(super) claim_purposes: Vec<HistoryClaimPurpose>,
    pub(super) pending_process: Option<u8>,
}

impl Script {
    pub(super) fn source_failure() -> Self {
        Self {
            source_error: true,
            ..Self::default()
        }
    }

    pub(super) fn with(payloads: Vec<Payload>) -> Self {
        Self {
            payloads,
            ..Self::default()
        }
    }

    pub(super) fn claimed_count(&self) -> usize {
        self.claimed_ids.len()
    }

    pub(super) fn was_claimed(&self, id: u8) -> bool {
        self.claimed_ids.contains(&id)
    }
}

impl HistoryPollCycleIo for Script {
    type Payload = Payload;
    type Item = Work;
    type Admitted = Admitted;
    type SourceError = &'static str;
    type AdaptationError = &'static str;
    type ClaimError = &'static str;
    type ProcessError = &'static str;

    fn fetch(
        &mut self,
        channel_id: u64,
        limit: usize,
    ) -> BoxHistoryPollFuture<'_, Vec<Payload>, &'static str> {
        self.events.push(format!("fetch:{channel_id}:{limit}"));
        let result = if self.source_error {
            Err("source")
        } else {
            Ok(std::mem::take(&mut self.payloads))
        };
        Box::pin(std::future::ready(result))
    }

    fn adapt(&mut self, payload: Payload) -> Result<HistoryBatchItem<Work>, &'static str> {
        self.events.push(format!("adapt:{}", payload.0));
        if self.adaptation_error == Some(payload.0) {
            return Err("adaptation");
        }
        let kind = match payload.2 {
            PayloadKind::Ignore => HistoryPollItem::Ignore,
            PayloadKind::Candidate => HistoryPollItem::Candidate(Work(payload.0, payload.3)),
        };
        Ok(HistoryBatchItem {
            watermark: HistoryWatermark::from_message(payload.1, u64::from(payload.0)),
            kind,
        })
    }

    fn claim(
        &mut self,
        item: Work,
        purpose: HistoryClaimPurpose,
    ) -> Result<HistoryClaimOutcome<Admitted>, &'static str> {
        self.claim_purposes.push(purpose);
        self.events.push(format!("claim:{}", item.0));
        if self.claim_error == Some(item.0) {
            return Err("claim");
        }
        if self.lost_claims.contains(&item.0) {
            return Ok(HistoryClaimOutcome::Lost);
        }
        if self.claimed_ids.insert(item.0) {
            Ok(HistoryClaimOutcome::Won(Admitted(item)))
        } else {
            Ok(HistoryClaimOutcome::Lost)
        }
    }

    fn process(&mut self, admitted: Admitted) -> BoxHistoryPollFuture<'_, (), &'static str> {
        self.events.push(format!("process:{}", admitted.0.0));
        if self.pending_process == Some(admitted.0.0) {
            return Box::pin(async move {
                std::future::pending::<()>().await;
                drop(admitted);
                Ok(())
            });
        }
        let result = if self.process_error == Some(admitted.0.0) {
            Err("process")
        } else {
            Ok(())
        };
        Box::pin(async move {
            drop(admitted.0.1);
            result
        })
    }
}

pub(super) fn ignored(id: u8, micros: i64) -> Payload {
    Payload(
        id,
        micros,
        PayloadKind::Ignore,
        format!("item-{id}").into_boxed_str(),
    )
}

pub(super) fn candidate(id: u8, micros: i64) -> Payload {
    Payload(
        id,
        micros,
        PayloadKind::Candidate,
        format!("item-{id}").into_boxed_str(),
    )
}

pub(super) async fn cycle<State: HistoryPollCycleState>(
    state: &mut State,
    started_at: i64,
    script: &mut Script,
) -> HistoryPollRunResult<Script> {
    run_history_poll_cycle(
        state,
        CHANNEL,
        PollStartedAt::from_normalized_utc_micros(started_at),
        script,
    )
    .await
}

pub(super) fn events(script: &Script) -> String {
    script.events.join(",")
}

pub(super) fn key(micros: i64, id: u64) -> HistoryWatermark {
    HistoryWatermark::from_message(micros, id).expect("valid key")
}

pub(super) struct StaleCommit(HistoryPollState);

impl StaleCommit {
    pub(super) fn new() -> Self {
        Self(HistoryPollState::default())
    }

    pub(super) fn watermark(&self) -> Option<HistoryWatermark> {
        self.0.watermark(CHANNEL)
    }
}

impl HistoryPollCycleState for StaleCommit {
    fn begin(
        &mut self,
        channel_id: u64,
        started_at: PollStartedAt,
    ) -> Result<HistoryPollCycleToken, HistoryPollStateError> {
        self.0.begin(channel_id, started_at)
    }

    fn propose<T>(
        &self,
        cycle: &HistoryPollCycleToken,
        items: Vec<HistoryBatchItem<T>>,
    ) -> Result<HistoryPollProposal<T>, HistoryPollStateError> {
        self.0.propose(cycle, items)
    }

    fn commit(
        &mut self,
        channel_id: u64,
        _token: &HistoryPollCommitToken,
    ) -> Result<(), HistoryPollCommitError> {
        Err(HistoryPollCommitError::Stale { channel_id })
    }
}
