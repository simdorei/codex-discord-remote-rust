use std::collections::BTreeMap;
use std::fs;
use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use cdr_store::processed::is_processed;
use twilight_model::channel::Message;

use super::{ErrorReportTarget, dispatch_message_create, prepare_message_create_at};
use crate::config::{CliOptions, RuntimeConfig};
use crate::discord_runtime::DiscordRuntimeError;
use crate::message_plan::MessagePlanError;
use crate::message_worker::{MessageAdmissionError, MessageWorkerError};

#[derive(Default)]
struct Effects {
    attachment: AtomicUsize,
    action: AtomicUsize,
    reply: AtomicUsize,
    error_report: AtomicUsize,
}

impl Effects {
    fn assert_zero(&self) {
        assert_eq!(self.attachment.load(Ordering::SeqCst), 0);
        assert_eq!(self.action.load(Ordering::SeqCst), 0);
        assert_eq!(self.reply.load(Ordering::SeqCst), 0);
        assert_eq!(self.error_report.load(Ordering::SeqCst), 0);
    }
}

pub(super) fn config() -> RuntimeConfig {
    RuntimeConfig::from_map(
        &BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "test-token".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
        ]),
        CliOptions::default(),
    )
    .expect("valid test config")
}

pub(super) fn message(id: u64, content: &str) -> Message {
    serde_json::from_value(serde_json::json!({
        "attachments": [],
        "author": {"avatar": null, "bot": false, "discriminator": "0001",
            "id": "3", "username": "tester"},
        "channel_id": "2", "content": content, "edited_timestamp": null,
        "embeds": [], "id": id.to_string(), "mention_everyone": false,
        "mention_roles": [], "mentions": [], "pinned": false,
        "timestamp": "2020-02-02T02:02:02.020000+00:00", "tts": false, "type": 0
    }))
    .expect("valid gateway message fixture")
}

fn attach(message: &mut Message) {
    message.attachments = serde_json::from_value(serde_json::json!([{
        "content_type": "text/plain", "ephemeral": false,
        "filename": "never-downloaded.txt", "id": "1",
        "proxy_url": "http://127.0.0.1:9/never", "size": 1,
        "url": "http://127.0.0.1:9/never"
    }]))
    .expect("valid attachment fixture");
}

fn seen_at(database: &std::path::Path, message_id: i64) -> f64 {
    rusqlite::Connection::open(database)
        .expect("open database")
        .query_row(
            "SELECT seen_at FROM discord_processed_messages WHERE message_id = ?",
            [message_id],
            |row| row.get(0),
        )
        .expect("read claim timestamp")
}

#[tokio::test]
async fn mcb_01_ignored_attachment_stops_every_effect_before_admission() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let attachment_root = temp.path().join("attachments-must-not-exist");
    let mut ignored_config = config();
    ignored_config.enable_message_content = false;
    let mut ignored = message(301, "hello");
    attach(&mut ignored);
    let report_target = ErrorReportTarget::from_message(&ignored);
    let effects = Effects::default();

    dispatch_message_create(
        report_target,
        || {
            prepare_message_create_at(
                ignored,
                None,
                &ignored_config,
                &database,
                UNIX_EPOCH + Duration::from_secs(100),
            )
        },
        |_| {
            effects.attachment.fetch_add(1, Ordering::SeqCst);
            fs::create_dir_all(&attachment_root).expect("effect sentinel directory");
            effects.action.fetch_add(1, Ordering::SeqCst);
            effects.reply.fetch_add(1, Ordering::SeqCst);
            ready(Result::<(), MessageWorkerError>::Ok(()))
        },
        |_, _| {
            effects.error_report.fetch_add(1, Ordering::SeqCst);
            ready(())
        },
    )
    .await
    .expect("ignore is a successful terminal outcome");

    effects.assert_zero();
    assert!(!attachment_root.exists());
    assert!(!is_processed(&database, 301).expect("read replay state"));
}

#[tokio::test]
async fn mcb_02_downstream_failure_and_success_never_refresh_the_claim() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let failed = message(302, "fail after claim");
    let failed_target = ErrorReportTarget::from_message(&failed);
    let effects = Effects::default();

    dispatch_message_create(
        failed_target,
        || {
            prepare_message_create_at(
                failed,
                None,
                &config(),
                &database,
                UNIX_EPOCH + Duration::from_secs(100),
            )
        },
        |_| {
            effects.action.fetch_add(1, Ordering::SeqCst);
            ready(Err(MessageWorkerError::Plan(
                MessagePlanError::UnsupportedPrefix("injected-downstream"),
            )))
        },
        |target, _| {
            assert_eq!(target, failed_target);
            assert_eq!(target.channel_id.get(), 2);
            assert_eq!(target.message_id.get(), 302);
            effects.error_report.fetch_add(1, Ordering::SeqCst);
            ready(())
        },
    )
    .await
    .expect("ordinary downstream error is reported");
    let redelivered = message(302, "fail after claim");
    dispatch_message_create(
        ErrorReportTarget::from_message(&redelivered),
        || {
            prepare_message_create_at(
                redelivered,
                None,
                &config(),
                &database,
                UNIX_EPOCH + Duration::from_secs(200),
            )
        },
        |_| {
            effects.action.fetch_add(1, Ordering::SeqCst);
            ready(Ok(()))
        },
        |_, _| {
            effects.error_report.fetch_add(1, Ordering::SeqCst);
            ready(())
        },
    )
    .await
    .expect("redelivery is suppressed");
    assert_eq!(effects.action.load(Ordering::SeqCst), 1);
    assert_eq!(effects.error_report.load(Ordering::SeqCst), 1);
    assert!((seen_at(&database, 302) - 100.0).abs() < f64::EPSILON);

    let succeeded = message(303, "succeed after claim");
    dispatch_message_create(
        ErrorReportTarget::from_message(&succeeded),
        || {
            prepare_message_create_at(
                succeeded,
                None,
                &config(),
                &database,
                UNIX_EPOCH + Duration::from_mins(5),
            )
        },
        |_| ready(Ok(())),
        |_, _| ready(()),
    )
    .await
    .expect("success completes");
    assert!((seen_at(&database, 303) - 300.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn mcb_03_database_affinity_failure_is_fatal_and_never_reported() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database_a = temp.path().join("a.sqlite");
    let database_b = temp.path().join("b.sqlite");
    let input = message(304, "hello");
    let report_target = ErrorReportTarget::from_message(&input);
    let effects = Effects::default();

    let error = dispatch_message_create(
        report_target,
        || {
            prepare_message_create_at(
                input,
                None,
                &config(),
                &database_a,
                UNIX_EPOCH + Duration::from_secs(400),
            )
        },
        |admitted| {
            ready(
                admitted
                    .verify_database_affinity(&database_b)
                    .map(|_| effects.action.fetch_add(1, Ordering::SeqCst))
                    .map(|_| ())
                    .map_err(MessageWorkerError::Admission),
            )
        },
        |_, _| {
            effects.error_report.fetch_add(1, Ordering::SeqCst);
            ready(())
        },
    )
    .await
    .expect_err("database mismatch is an internal fatal error");

    assert!(matches!(
        error,
        DiscordRuntimeError::MessageAdmission(MessageAdmissionError::DatabaseMismatch { .. })
    ));
    effects.assert_zero();
    assert!(is_processed(&database_a, 304).expect("read database A"));
    assert!(!is_processed(&database_b, 304).expect("read database B"));
}
