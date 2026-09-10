use super::types::{
    ExpectedChannelState, HistoryBatchItem, HistoryItemDecision, HistoryPollItem, HistoryPollPhase,
    HistoryWatermark,
};

#[derive(Clone, Copy)]
pub(super) enum ItemDecisionMode {
    Prime(HistoryWatermark),
    Reprime,
    Incremental(HistoryWatermark),
}

pub(super) fn proposal_parameters(
    state: ExpectedChannelState,
    latest: Option<HistoryWatermark>,
) -> (HistoryPollPhase, HistoryWatermark, ItemDecisionMode) {
    match state {
        ExpectedChannelState::Priming {
            phase, boundary, ..
        } => {
            let next = latest.map_or(boundary, |value| value.max(boundary));
            let mode = if phase == HistoryPollPhase::Prime {
                ItemDecisionMode::Prime(boundary)
            } else {
                ItemDecisionMode::Reprime
            };
            (phase, next, mode)
        }
        ExpectedChannelState::Active {
            watermark: Some(previous),
            ..
        } => {
            let next = latest.map_or(previous, |value| value.max(previous));
            (
                HistoryPollPhase::Incremental,
                next,
                ItemDecisionMode::Incremental(previous),
            )
        }
        ExpectedChannelState::Active {
            watermark: None, ..
        } => unreachable!("begin converts a missing watermark into a reprime cycle"),
    }
}

pub(super) fn decide_items<T>(
    newest_first: Vec<HistoryBatchItem<T>>,
    mode: ItemDecisionMode,
) -> Vec<HistoryItemDecision<T>> {
    newest_first
        .into_iter()
        .rev()
        .map(|item| match item.kind {
            HistoryPollItem::Ignore => HistoryItemDecision::NoClaim,
            HistoryPollItem::Candidate(value) => candidate_decision(value, item.watermark, mode),
        })
        .collect()
}

fn candidate_decision<T>(
    value: T,
    watermark: Option<HistoryWatermark>,
    mode: ItemDecisionMode,
) -> HistoryItemDecision<T> {
    let Some(current) = watermark else {
        return HistoryItemDecision::NoClaim;
    };
    match mode {
        ItemDecisionMode::Prime(boundary) if current > boundary => {
            HistoryItemDecision::ClaimAndProcess(value)
        }
        ItemDecisionMode::Prime(_) | ItemDecisionMode::Reprime => {
            HistoryItemDecision::ClaimAndDiscard(value)
        }
        ItemDecisionMode::Incremental(boundary) if current > boundary => {
            HistoryItemDecision::ClaimAndProcess(value)
        }
        ItemDecisionMode::Incremental(_) => HistoryItemDecision::ClaimAndDiscard(value),
    }
}
