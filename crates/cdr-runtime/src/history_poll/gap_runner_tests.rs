use std::collections::HashSet;

use super::*;
use crate::history_poll::{BoxHistoryPollFuture, HistoryBatchItem};

enum PayloadKind {
    Ignore,
    Candidate,
}

struct Payload {
    id: u64,
    micros: i64,
    kind: PayloadKind,
}

struct Work(u64);
struct Admitted(Work);

#[derive(Default)]
struct Script {
    payloads: Vec<Payload>,
    events: Vec<String>,
    claims: HashSet<u64>,
    process_error: Option<u64>,
}

impl Script {
    fn with(payloads: Vec<Payload>) -> Self {
        Self {
            payloads,
            ..Self::default()
        }
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
        Box::pin(std::future::ready(Ok(std::mem::take(&mut self.payloads))))
    }

    fn adapt(&mut self, payload: Payload) -> Result<HistoryBatchItem<Work>, &'static str> {
        self.events.push(format!("adapt:{}", payload.id));
        let kind = match payload.kind {
            PayloadKind::Ignore => HistoryPollItem::Ignore,
            PayloadKind::Candidate => HistoryPollItem::Candidate(Work(payload.id)),
        };
        Ok(HistoryBatchItem {
            watermark: HistoryWatermark::from_message(payload.micros, payload.id),
            kind,
        })
    }

    fn claim(
        &mut self,
        item: Work,
        _purpose: HistoryClaimPurpose,
    ) -> Result<HistoryClaimOutcome<Admitted>, &'static str> {
        self.events.push(format!("claim:{}", item.0));
        Ok(if self.claims.insert(item.0) {
            HistoryClaimOutcome::Won(Admitted(item))
        } else {
            HistoryClaimOutcome::Lost
        })
    }

    fn process(&mut self, admitted: Admitted) -> BoxHistoryPollFuture<'_, (), &'static str> {
        let id = admitted.0.0;
        self.events.push(format!("process:{id}"));
        Box::pin(std::future::ready(if self.process_error == Some(id) {
            Err("process")
        } else {
            Ok(())
        }))
    }
}

fn candidate(id: u64, micros: i64) -> Payload {
    Payload {
        id,
        micros,
        kind: PayloadKind::Candidate,
    }
}

fn ignored(id: u64, micros: i64) -> Payload {
    Payload {
        id,
        micros,
        kind: PayloadKind::Ignore,
    }
}

fn key(micros: i64, id: u64) -> HistoryWatermark {
    HistoryWatermark::from_message(micros, id).expect("valid floor")
}

#[tokio::test]
async fn hgr_01_floor_is_inclusive_and_candidates_run_oldest_first() {
    let mut io = Script::with(vec![candidate(3, 103), candidate(2, 100), candidate(1, 99)]);

    let outcome = run_history_gap_recovery(7, key(100, 2), &mut io)
        .await
        .expect("recover gap");

    assert_eq!(outcome.coverage, HistoryGapCoverage::Reached);
    assert_eq!((outcome.no_claim, outcome.claim_attempted), (1, 2));
    assert_eq!((outcome.claim_won, outcome.processed), (2, 2));
    assert_eq!(
        io.events.join(","),
        "fetch:7:10,adapt:3,adapt:2,adapt:1,claim:2,process:2,claim:3,process:3"
    );
}

#[tokio::test]
async fn hgr_02_full_page_above_floor_stays_incomplete_after_partial_recovery() {
    let payloads = (1..=10)
        .rev()
        .map(|id| candidate(id, 100 + i64::try_from(id).unwrap()))
        .collect();
    let mut io = Script::with(payloads);

    let outcome = run_history_gap_recovery(7, key(100, 99), &mut io)
        .await
        .expect("partial recovery still reports its coverage");

    assert_eq!(outcome.coverage, HistoryGapCoverage::Incomplete);
    assert_eq!(outcome.fetched, 10);
    assert_eq!(outcome.processed, 10);
}

#[tokio::test]
async fn hgr_03_full_page_reaching_exact_floor_is_complete() {
    let payloads = (1..=10)
        .rev()
        .map(|id| candidate(id, 100 + i64::try_from(id).unwrap()))
        .collect();
    let mut io = Script::with(payloads);

    let outcome = run_history_gap_recovery(7, key(101, 1), &mut io)
        .await
        .expect("exact floor is covered");

    assert_eq!(outcome.coverage, HistoryGapCoverage::Reached);
    assert_eq!(outcome.processed, 10);
}

#[tokio::test]
async fn hgr_04_short_page_proves_history_exhaustion_without_an_exact_floor_item() {
    let mut io = Script::with(vec![candidate(3, 103), ignored(2, 102)]);

    let outcome = run_history_gap_recovery(7, key(100, 1), &mut io)
        .await
        .expect("short page reaches channel history start");

    assert_eq!(outcome.coverage, HistoryGapCoverage::Reached);
    assert_eq!((outcome.no_claim, outcome.processed), (1, 1));
}

#[tokio::test]
async fn hgr_05_process_failure_is_not_misreported_as_recovered() {
    let mut io = Script::with(vec![candidate(1, 101)]);
    io.process_error = Some(1);

    assert_eq!(
        run_history_gap_recovery(7, key(101, 1), &mut io).await,
        Err(HistoryGapRecoveryError::Process("process"))
    );
    assert!(io.claims.contains(&1));
}
