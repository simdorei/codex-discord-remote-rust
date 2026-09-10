use cdr_runtime::discord_dispatch::{
    BoxDiscordFuture, DispatchOutcome, InteractionDispatcher, InteractionTransport,
};
use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use std::sync::Arc;
#[path = "support/action_app_server.rs"]
mod app;
#[path = "support/retract_cancelled_worker.rs"]
mod cancelled_worker;
#[path = "support/action_target.rs"]
mod target;
use twilight_model::{
    http::interaction::InteractionResponse,
    id::{Id, marker::InteractionMarker},
};

struct Ack;
impl InteractionTransport for Ack {
    fn acknowledge<'a>(
        &'a self,
        _: Id<InteractionMarker>,
        _: &'a str,
        _: &'a InteractionResponse,
    ) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn update<'a>(&'a self, _: &'a str, _: &'a str) -> BoxDiscordFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn actual_slash_admission_freezes_mapped_target_for_early_cancellation() {
    for name in ["ask", "interview"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start_fake_server(&temp, &log).await);
        let backend = Arc::new(target::FakeBackend::default());
        let executor = Arc::new(
            target::executor(
                &temp,
                db.clone(),
                Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
                backend.clone(),
            )
            .with_server(server.clone()),
        );
        cdr_store::mapping::upsert_thread(&db, "original", "project", "title", 100, 42, 1.0)
            .unwrap();
        seed_older(&db);
        let work = stage_slash(&db, name).await;
        let before = cdr_store::ingress::get(&db, &work.custody_ingress_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            before.target_thread_id.as_deref(),
            Some("original"),
            "{name}: actual ingress, not a manually patched fixture"
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let runners = executor
            .execute(CommandAction::Runners, 42, 3)
            .await
            .unwrap();
        assert!(runners.text.contains("unowned_ingress: 1"));
        let retract = executor
            .execute(CommandAction::Retract { reference: None }, 42, 3)
            .await
            .unwrap();
        assert!(retract.text.contains("ingress:interaction:201"));
        assert!(
            cdr_store::prompt_intake::get_prompt_intake(&db, "older")
                .unwrap()
                .is_some()
        );
        let after = cdr_store::ingress::get(&db, &work.custody_ingress_id)
            .unwrap()
            .unwrap();
        assert_eq!(after.phase, "cancelled");
        assert_eq!(after.payload, before.payload);
        assert!(
            !cdr_store::ingress::begin_execution(
                &db,
                &work.custody_ingress_id,
                "processing",
                None,
                now + 1.0
            )
            .unwrap()
        );
        cancelled_worker::release(work, executor, server.clone()).await;
        server.close().await.unwrap();
        assert!(backend.starts.lock().await.is_empty());
        assert!(backend.resumes.lock().await.is_empty());
        assert!(backend.forks.lock().await.is_empty());
        assert!(
            app::rpc_log(&log)
                .iter()
                .all(|value| value["method"] == "initialize")
        );
        assert_eq!(
            cdr_store::ingress::get(&db, "interaction:201")
                .unwrap()
                .unwrap()
                .phase,
            "cancelled"
        );
    }
}

async fn stage_slash(
    db: &std::path::Path,
    name: &str,
) -> cdr_runtime::discord_dispatch::InboundInteractionWork {
    let (send, mut recv) = tokio::sync::mpsc::channel(1);
    let dispatcher = InteractionDispatcher::new(
        Arc::new(Ack),
        cdr_discord::interaction_access::InteractionAccessPolicy {
            allow_all_channels: true,
            ..Default::default()
        },
        false,
        send,
        db,
    );
    let event = serde_json::from_value(serde_json::json!({
            "application_id":"2", "authorizing_integration_owners":{}, "channel_id":"42",
            "data":{"id":"1","name":name,"type":1,"options":[{"name":"prompt","type":3,"value":"취소할 대기 요청"}]},
            "entitlements":[],"id":"201","locale":"en-US","token":"fixture","type":2,
            "user":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},"version":1
        })).unwrap();
    assert_eq!(
        dispatcher
            .dispatch(
                &event,
                tokio::time::Instant::now(),
                cdr_discord::gateway::ingress::InteractionIngressTag::Normal
            )
            .await
            .unwrap(),
        DispatchOutcome::Queued
    );
    recv.recv().await.unwrap()
}

fn seed_older(db: &std::path::Path) {
    cdr_store::prompt_intake::admit_prompt_intake(
        db,
        cdr_store::prompt_intake::NewPromptIntake {
            job_id: "older",
            target_thread_id: "original",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(100),
            raw_prompt: "older request remains",
            auto_queue_when_busy: true,
            require_current_mirror: true,
            created_at: 1.0,
        },
    )
    .unwrap();
}
