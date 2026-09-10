//! Only history fetching is substituted; adaptation, custody and processing are real.
use super::*;
use crate::{
    app_backend::AppServerTurnBackend,
    history_poll::{
        HistoryPollState, PollStartedAt, run_history_gap_recovery, run_history_poll_cycle,
    },
    test_support::{app_fixture, new_reply_fixture},
};
use std::sync::atomic::Ordering;

struct PageIo<'a> {
    inner: DiscordHistoryCycleIo<'a, AppServerTurnBackend>,
    page: Vec<Message>,
}

impl HistoryPollCycleIo for PageIo<'_> {
    type Payload = Message;
    type Item = <DiscordHistoryCycleIo<'static, AppServerTurnBackend> as HistoryPollCycleIo>::Item;
    type Admitted = HistoryAdmittedMessage;
    type SourceError = DiscordHistorySourceError;
    type AdaptationError = MessageAdmissionError;
    type ClaimError = MessageAdmissionError;
    type ProcessError = MessageAdmissionError;
    fn fetch(
        &mut self,
        _: u64,
        _: usize,
    ) -> BoxHistoryPollFuture<'_, Vec<Message>, Self::SourceError> {
        Box::pin(std::future::ready(Ok(std::mem::take(&mut self.page))))
    }
    fn adapt(
        &mut self,
        item: Message,
    ) -> Result<HistoryBatchItem<Self::Item>, Self::AdaptationError> {
        self.inner.adapt(item)
    }
    fn claim(
        &mut self,
        item: Self::Item,
        purpose: HistoryClaimPurpose,
    ) -> Result<HistoryClaimOutcome<Self::Admitted>, Self::ClaimError> {
        self.inner.claim(item, purpose)
    }
    fn process(
        &mut self,
        item: Self::Admitted,
    ) -> BoxHistoryPollFuture<'_, (), Self::ProcessError> {
        self.inner.process(item)
    }
}

fn message(id: u64, content: &str) -> Message {
    serde_json::from_value(serde_json::json!({
        "attachments":[],"author":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},
        "channel_id":"100","content":content,"edited_timestamp":null,"embeds":[],"id":id.to_string(),
        "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
        "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
    })).unwrap()
}

fn mention_required_config() -> RuntimeConfig {
    let mut config = RuntimeConfig::from_map(
        &std::collections::BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "fixture-token".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
        ]),
        crate::config::CliOptions::default(),
    )
    .unwrap();
    config.plain_ask_mention_user_ids.insert(777);
    config
}

async fn same_page(gap: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, gate) =
        new_reply_fixture::setup_with_channels(&temp, vec![100, 43]).await;
    cdr_store::mapping::upsert_thread(
        fixture.executor.mirror_db(),
        "thread-a",
        &temp.path().to_string_lossy(),
        "other",
        100,
        44,
        1.0,
    )
    .unwrap();
    assert!(
        cdr_store::mapping::mirrored_thread_id(fixture.executor.mirror_db(), Some(100))
            .unwrap()
            .is_none()
    );
    let config = mention_required_config();
    let mut context = fixture.context(temp.path());
    context.config = &config;
    let mut io = PageIo {
        inner: DiscordHistoryCycleIo::new(
            context,
            InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            None,
            SystemTime::now(),
        ),
        page: vec![],
    };
    let mut state = HistoryPollState::default();
    let started = PollStartedAt::from_normalized_utc_micros(0);
    run_history_poll_cycle(&mut state, 100, started, &mut io)
        .await
        .unwrap();
    io.page = vec![
        message(803, "unmentioned tail"),
        message(802, "first"),
        message(801, "!new"),
    ];
    gate.release.send(()).unwrap();
    if gap {
        let floor = HistoryWatermark::from_message(message(801, "!new").timestamp.as_micros(), 801)
            .unwrap();
        run_history_gap_recovery(100, floor, &mut io).await.unwrap();
    } else {
        run_history_poll_cycle(&mut state, 100, started, &mut io)
            .await
            .unwrap();
        assert_eq!(
            state.watermark(100),
            HistoryWatermark::from_message(message(803, "").timestamp.as_micros(), 803)
        );
    }
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "thread/start").count(),
        1,
        "same-page !new prompt was lost"
    );
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "turn/start").count(),
        1
    );
    assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    assert_eq!(
        posts.len(),
        2,
        "one reservation notice and one original prompt echo"
    );
    assert_eq!(
        posts[1]["content"],
        "In progress\nmessage: first\n새 대화: <#43>"
    );
    assert!(posts.iter().all(|p| p["test_channel"] == 100));
    let db = fixture.executor.mirror_db();
    assert!(
        cdr_store::ingress::get(db, "message:802")
            .unwrap()
            .is_some()
    );
    assert!(
        cdr_store::ingress::get(db, "message:803")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn incremental_page_new_then_plain_prompt_is_not_lost_before_watermark_commit() {
    same_page(false).await;
}

#[tokio::test]
async fn gap_page_new_then_plain_prompt_is_not_lost() {
    same_page(true).await;
}

#[tokio::test]
async fn priming_old_new_does_not_create_a_live_reservation() {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, gate) =
        new_reply_fixture::setup_with_channels(&temp, vec![100, 43]).await;
    let mut io = PageIo {
        inner: DiscordHistoryCycleIo::new(
            fixture.context(temp.path()),
            InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            None,
            SystemTime::now(),
        ),
        page: vec![message(801, "!new")],
    };
    let mut state = HistoryPollState::default();
    run_history_poll_cycle(
        &mut state,
        100,
        PollStartedAt::from_normalized_utc_micros(i64::MAX),
        &mut io,
    )
    .await
    .unwrap();
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    assert!(gate.task.await.unwrap().is_empty());
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    let db = fixture.executor.mirror_db();
    assert!(cdr_store::processed::is_processed(db, 801).unwrap());
    assert!(
        cdr_store::ingress::pending_new_prompt(db, 100, 3, 802)
            .unwrap()
            .is_none(),
        "discard is not authority to arm !new"
    );
    assert!(
        cdr_store::ingress::get(db, "message:801")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn priming_old_plain_prompt_does_not_consume_a_live_reservation() {
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, gate) =
        new_reply_fixture::setup_with_channels(&temp, vec![100, 43]).await;
    let mut io = PageIo {
        inner: DiscordHistoryCycleIo::new(
            fixture.context(temp.path()),
            InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            None,
            SystemTime::now(),
        ),
        page: vec![message(801, "historical prompt")],
    };
    gate.release.send(()).unwrap();
    let HistoryPollItem::Candidate(arm) = io.adapt(message(800, "!new")).unwrap().kind else {
        panic!("arm candidate")
    };
    let HistoryClaimOutcome::Won(arm) = io.claim(arm, HistoryClaimPurpose::Process).unwrap() else {
        panic!("arm claim")
    };
    io.process(arm).await.unwrap();
    let mut state = HistoryPollState::default();
    run_history_poll_cycle(
        &mut state,
        100,
        PollStartedAt::from_normalized_utc_micros(i64::MAX),
        &mut io,
    )
    .await
    .unwrap();
    let db = fixture.executor.mirror_db();
    assert_eq!(
        cdr_store::ingress::pending_new_prompt(db, 100, 3, 802)
            .unwrap()
            .as_deref(),
        Some("message:800")
    );
    assert!(cdr_store::processed::is_processed(db, 801).unwrap());
    assert!(
        cdr_store::ingress::get(db, "message:801")
            .unwrap()
            .is_none()
    );
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    assert_eq!(gate.task.await.unwrap().len(), 1);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    assert!(
        app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .all(|r| r["method"] != "thread/start" && r["method"] != "turn/start")
    );
}
