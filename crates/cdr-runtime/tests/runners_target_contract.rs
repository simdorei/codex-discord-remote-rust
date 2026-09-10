use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use cdr_store::{prompt_intake, queue};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/action_app_server.rs"]
mod app;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn target_view_distinguishes_owned_active_from_stored_quarantine_without_control_rpc() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start_fake_server(&temp, &log).await);
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0).unwrap();
    let executor = target::executor(
        &temp,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(target::FakeBackend::default()),
    )
    .with_server(server.clone());
    for (id, thread, channel) in [("quarantined", "thread-b", 42), ("foreign", "thread-a", 43)] {
        queue::enqueue(
            &db,
            queue::NewQueueJob {
                job_id: id,
                target_thread_id: thread,
                channel_id: channel,
                owner_user_id: Some(3),
                discord_message_id: None,
                app_server_generation: 1,
                prompt: "private queued prompt",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
    }
    cdr_store::schema::open_initialized(&db).unwrap().execute(
        "UPDATE codex_turn_queue SET state='running',turn_id='cdr-quarantined:fixture',last_error='[cdr-rust:app-server-fork-quarantine:v1] fixture' WHERE job_id='quarantined'",[],
    ).unwrap();
    prompt_intake::admit_prompt_intake(
        &db,
        prompt_intake::NewPromptIntake {
            job_id: "intake",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            raw_prompt: "private intake prompt",
            auto_queue_when_busy: true,
            require_current_mirror: true,
            created_at: 2.0,
        },
    )
    .unwrap();
    server
        .execute(
            cdr_app_server::requests::AppRequest {
                method: "test/active-turn",
                params: json!({"threadId":"thread-b","turnId":"owned-turn"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    let before_jobs = queue::list(&db).unwrap();
    let before_intakes = prompt_intake::list_prompt_intakes(&db).unwrap();
    let result = executor
        .execute(CommandAction::Runners, 42, 3)
        .await
        .unwrap();
    server.close().await.unwrap();
    assert!(
        result.text.contains("owned_active_turn: owned-turn"),
        "{}",
        result.text
    );
    let current = result.text.split("Current target work").nth(1).unwrap();
    for expected in [
        "target: thread-b",
        "queued: 0",
        "running_records: 0",
        "quarantined_records: 1",
        "intake: 1",
    ] {
        assert!(current.contains(expected), "{expected}: {current}");
    }
    assert!(!result.text.contains("private"));
    assert_eq!(queue::list(&db).unwrap(), before_jobs);
    assert_eq!(
        prompt_intake::list_prompt_intakes(&db).unwrap(),
        before_intakes
    );
    let calls = app::rpc_log(&log);
    assert!(calls.iter().all(|v| matches!(
        v["method"].as_str(),
        Some("initialize" | "test/active-turn")
    )));
    let disconnected = executor
        .execute(CommandAction::Runners, 42, 3)
        .await
        .unwrap();
    assert!(disconnected.text.contains("owned_active_turn: unknown"));
    assert!(!disconnected.text.contains("owned_active_turn: owned-turn"));
}
