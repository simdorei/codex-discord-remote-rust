use std::future::ready;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use twilight_model::channel::Message;

use super::*;
use crate::message_plan::MessagePlanError;

fn target() -> ErrorReportTarget {
    let message: Message = serde_json::from_value(serde_json::json!({
        "attachments": [],
        "author": {"avatar": null, "bot": false, "discriminator": "0001",
            "id": "3", "username": "tester"},
        "channel_id": "2", "content": "hello", "edited_timestamp": null,
        "embeds": [], "id": "42", "mention_everyone": false,
        "mention_roles": [], "mentions": [], "pinned": false,
        "timestamp": "2020-02-02T02:02:02.020000+00:00", "tts": false, "type": 0
    }))
    .expect("valid message fixture");
    ErrorReportTarget::from_message(&message)
}

#[tokio::test]
async fn mpb_10_ordinary_worker_error_reports_exactly_once_and_is_terminal_success() {
    let reports = AtomicUsize::new(0);

    process_with_error_report(
        target(),
        "owned work",
        |work| {
            assert_eq!(work, "owned work");
            ready(Err(MessageWorkerError::Plan(
                MessagePlanError::UnsupportedPrefix("ordinary"),
            )))
        },
        |actual, error| {
            assert_eq!(actual, target());
            assert!(error.to_string().contains("ordinary"));
            reports.fetch_add(1, Ordering::SeqCst);
            ready(())
        },
    )
    .await
    .expect("reported worker errors are terminal success");

    assert_eq!(reports.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mpb_11_database_mismatch_is_fatal_and_skips_error_report() {
    let claimed_database = PathBuf::from("claimed.sqlite");
    let context_database = PathBuf::from("context.sqlite");
    let error = process_with_error_report(
        target(),
        (),
        |()| {
            ready(Err(MessageWorkerError::Admission(
                MessageAdmissionError::DatabaseMismatch {
                    claimed_database: claimed_database.clone(),
                    context_database: context_database.clone(),
                },
            )))
        },
        |_, _| -> std::future::Ready<()> { panic!("database mismatch must not be reported") },
    )
    .await
    .expect_err("database mismatch remains fatal");

    match error {
        MessageAdmissionError::DatabaseMismatch {
            claimed_database: claimed,
            context_database: context,
        } => {
            assert_eq!(claimed, claimed_database);
            assert_eq!(context, context_database);
        }
        other => panic!("unexpected fatal error: {other:?}"),
    }
}

#[tokio::test]
async fn mpb_12_success_never_enters_the_error_report_path() {
    process_with_error_report(
        target(),
        Box::new(7_u8),
        |work| {
            assert_eq!(*work, 7);
            ready(Ok(()))
        },
        |_, _| -> std::future::Ready<()> { panic!("success must not be reported") },
    )
    .await
    .expect("worker success passes the shared boundary");
}
