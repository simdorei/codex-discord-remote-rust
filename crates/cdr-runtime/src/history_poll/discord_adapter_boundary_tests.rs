use std::convert::Infallible;
use std::future::{pending, ready};
use std::path::PathBuf;

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use super::*;
use crate::history_poll::{
    HistoryPollRunError, HistoryPollState, PollStartedAt, run_history_poll_cycle,
};
use crate::message_plan::MessagePlanError;
use crate::message_worker::{MessageWorkerError, process_with_error_report};

#[derive(Clone, Copy)]
enum ProcessMode {
    OrdinaryError,
    DatabaseMismatch,
    Pending,
}

struct BoundaryIo {
    mode: ProcessMode,
    claimed: bool,
    reports: usize,
}

impl BoundaryIo {
    const fn new(mode: ProcessMode) -> Self {
        Self {
            mode,
            claimed: false,
            reports: 0,
        }
    }
}

impl HistoryPollCycleIo for BoundaryIo {
    type Payload = ();
    type Item = ();
    type Admitted = ();
    type SourceError = Infallible;
    type AdaptationError = Infallible;
    type ClaimError = Infallible;
    type ProcessError = MessageAdmissionError;

    fn fetch(
        &mut self,
        _channel_id: u64,
        _limit: usize,
    ) -> BoxHistoryPollFuture<'_, Vec<()>, Infallible> {
        Box::pin(ready(Ok(vec![()])))
    }

    fn adapt(&mut self, (): ()) -> Result<HistoryBatchItem<()>, Infallible> {
        Ok(HistoryBatchItem {
            watermark: HistoryWatermark::from_message(101, 42),
            kind: HistoryPollItem::Candidate(()),
        })
    }

    fn claim(
        &mut self,
        (): (),
        _purpose: HistoryClaimPurpose,
    ) -> Result<HistoryClaimOutcome<()>, Infallible> {
        self.claimed = true;
        Ok(HistoryClaimOutcome::Won(()))
    }

    fn process(&mut self, (): ()) -> BoxHistoryPollFuture<'_, (), MessageAdmissionError> {
        let mode = self.mode;
        let reports = &mut self.reports;
        let target = ErrorReportTarget {
            channel_id: Id::<ChannelMarker>::new(7),
            message_id: Id::<MessageMarker>::new(42),
        };
        Box::pin(async move {
            match mode {
                ProcessMode::OrdinaryError => {
                    process_with_error_report(
                        target,
                        (),
                        |()| {
                            ready(Err(MessageWorkerError::Plan(
                                MessagePlanError::UnsupportedPrefix("ordinary"),
                            )))
                        },
                        |_, _| {
                            *reports += 1;
                            ready(())
                        },
                    )
                    .await
                }
                ProcessMode::DatabaseMismatch => {
                    process_with_error_report(
                        target,
                        (),
                        |()| {
                            ready(Err(MessageWorkerError::Admission(
                                MessageAdmissionError::DatabaseMismatch {
                                    claimed_database: PathBuf::from("claimed.sqlite"),
                                    context_database: PathBuf::from("context.sqlite"),
                                },
                            )))
                        },
                        |_, _| {
                            *reports += 1;
                            ready(())
                        },
                    )
                    .await
                }
                ProcessMode::Pending => {
                    process_with_error_report(
                        target,
                        (),
                        |()| async {
                            pending::<()>().await;
                            Ok(())
                        },
                        |_, _| {
                            *reports += 1;
                            ready(())
                        },
                    )
                    .await
                }
            }
        })
    }
}

fn started() -> PollStartedAt {
    PollStartedAt::from_normalized_utc_micros(100)
}

#[tokio::test]
async fn dha_07_reported_worker_error_commits_the_channel_cursor() {
    let mut state = HistoryPollState::default();
    let mut io = BoundaryIo::new(ProcessMode::OrdinaryError);

    let outcome = run_history_poll_cycle(&mut state, 7, started(), &mut io)
        .await
        .expect("reported worker error is terminal");

    assert_eq!(outcome.processed, 1);
    assert!(io.claimed);
    assert_eq!(io.reports, 1);
    assert_eq!(state.watermark(7), HistoryWatermark::from_message(101, 42));
}

#[tokio::test]
async fn dha_08_database_mismatch_skips_report_and_cursor_commit() {
    let mut state = HistoryPollState::default();
    let mut io = BoundaryIo::new(ProcessMode::DatabaseMismatch);

    let error = run_history_poll_cycle(&mut state, 7, started(), &mut io)
        .await
        .expect_err("database mismatch remains fatal");

    assert!(matches!(
        error,
        HistoryPollRunError::Process(MessageAdmissionError::DatabaseMismatch { .. })
    ));
    assert!(io.claimed);
    assert_eq!(io.reports, 0);
    assert_eq!(state.watermark(7), None);
}

#[tokio::test]
async fn dha_09_cancellation_keeps_claim_without_reporting_or_committing() {
    let mut state = HistoryPollState::default();
    let mut io = BoundaryIo::new(ProcessMode::Pending);
    let mut run = Box::pin(run_history_poll_cycle(&mut state, 7, started(), &mut io));

    tokio::select! {
        biased;
        result = &mut run => panic!("unexpected completion: {result:?}"),
        () = ready(()) => {}
    }
    drop(run);

    assert!(io.claimed);
    assert_eq!(io.reports, 0);
    assert_eq!(state.watermark(7), None);
}
