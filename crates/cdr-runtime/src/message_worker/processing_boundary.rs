use std::future::Future;

use cdr_discord::http::DiscordHttp;
use twilight_model::{
    channel::Message,
    id::{
        Id,
        marker::{ChannelMarker, MessageMarker},
    },
};

use super::{MessageAdmissionError, MessageWorkerError};
use crate::message_worker::reply_delivery::{MessageReplyKind, send_reply_once};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ErrorReportTarget {
    pub(crate) channel_id: Id<ChannelMarker>,
    pub(crate) message_id: Id<MessageMarker>,
}

impl ErrorReportTarget {
    pub(crate) const fn from_message(message: &Message) -> Self {
        Self {
            channel_id: message.channel_id,
            message_id: message.id,
        }
    }
}

pub(crate) async fn process_with_error_report<Work, Process, ProcessFuture, Report, ReportFuture>(
    target: ErrorReportTarget,
    work: Work,
    process: Process,
    report: Report,
) -> Result<(), MessageAdmissionError>
where
    Process: FnOnce(Work) -> ProcessFuture,
    ProcessFuture: Future<Output = Result<(), MessageWorkerError>>,
    Report: FnOnce(ErrorReportTarget, MessageWorkerError) -> ReportFuture,
    ReportFuture: Future<Output = ()>,
{
    match process(work).await {
        Ok(()) => Ok(()),
        Err(MessageWorkerError::Admission(error)) if error.is_database_mismatch() => Err(error),
        Err(error) => {
            report(target, error).await;
            Ok(())
        }
    }
}

pub(crate) async fn report_processing_error(
    db: &std::path::Path,
    api: &DiscordHttp,
    target: ErrorReportTarget,
    error: MessageWorkerError,
) {
    eprintln!("on_message_error: {error}");
    if matches!(error, MessageWorkerError::KnownOutcomeNotification(_)) {
        // The original refusal is already durable. A second-key ERROR would
        // create a duplicate or conceal an uncertain delivery from this attempt.
        return;
    }
    let report = send_reply_once(
        db,
        api,
        target.channel_id,
        target.message_id,
        MessageReplyKind::ErrorReport,
        &format!("ERROR: {error}"),
        &[],
    )
    .await;
    if let Err(report_error) = report {
        eprintln!("on_message_error_report_failed: {report_error}");
    }
}

#[cfg(test)]
#[path = "processing_boundary_tests.rs"]
mod tests;
