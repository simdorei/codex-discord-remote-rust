use cdr_app_server::requests::AppRequest;
use cdr_discord::{
    components::{BusyAction, ComponentId},
    interaction::RoutedWork,
};
use cdr_runtime::{
    action_executor::{ActionContext, ActionExecutor, ActionUi},
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    command_plan::plan_slash,
    component_worker::{busy_ready_marker, confirmation_ready, handle_component_work},
    queue_runner::QueueCoordinator,
};
use cdr_store::{claims, ingress, mapping, prompt_intake, queue};
use std::{sync::Arc, time::Duration};
#[path = "support/action_app_server.rs"]
mod app;
#[path = "support/approval_click.rs"]
mod component;
#[path = "../src/completion_worker/goal_mirror_http.rs"]
mod http_gate;
#[path = "support/mapped_slash.rs"]
mod slash;

#[tokio::test]
async fn actual_mapped_slash_busy_queue_preserves_original_route() {
    for name in ["ask", "interview"] {
        for change in [0, 1, 2] {
            check(name, change).await;
        }
    }
}

async fn check(name: &str, change: u8) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start_fake_server(&temp, &log).await);
    activate(&server).await;
    let executor = Arc::new(
        ActionExecutor::new(
            temp.path().join("state.sqlite"),
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            Arc::new(QueueCoordinator::new(
                db.clone(),
                Arc::new(AppServerTurnBackend::new(server.clone())),
            )),
        )
        .with_server(server.clone()),
    );
    mapping::upsert_thread(&db, "A", "p", "a", 10, 42, 1.0).unwrap();
    let work = slash::stage(&db, name).await;
    let original = ingress::get(&db, &work.custody_ingress_id)
        .unwrap()
        .unwrap();
    assert_eq!(original.target_thread_id.as_deref(), Some("A"));
    assert!(
        ingress::begin_execution(&db, &work.custody_ingress_id, "processing", None, 2.0).unwrap()
    );
    let RoutedWork::Slash(invocation) = &work.work else {
        panic!("expected slash")
    };
    let result = executor
        .execute_with_context(
            plan_slash(invocation).unwrap(),
            ActionContext {
                channel_id: 42,
                user_id: 3,
                discord_message_id: Some(101),
                auto_queue_when_busy: false,
            },
        )
        .await
        .unwrap();
    let Some(ActionUi::Busy { choice_id, .. }) = result.ui else {
        panic!("expected busy choice")
    };
    let choice = claims::get_busy_choice(&db, &choice_id, 0.0)
        .unwrap()
        .unwrap();
    assert_eq!(choice.target_thread_id.as_deref(), Some("A"));
    // change=1: between display and click; change=2: after pre-ACK button snapshot.
    if change == 1 {
        move_mapping(&db);
    }
    let click = component::click(&db, 42, 3, &format!("codex_busy:{choice_id}:queue")).await;
    if change == 2 {
        move_mapping(&db);
    }
    assert_eq!(click.authorized_busy_choice.as_ref(), Some(&choice));
    if change == 0 {
        let button = ComponentId::Busy {
            choice_id: choice_id.clone(),
            action: BusyAction::Queue,
        };
        handle_component_work(&click, &button, &executor, &server)
            .await
            .unwrap();
        let jobs = queue::list(&db).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].target_thread_id, "A");
        assert_eq!(jobs[0].state, queue::QueueJobState::Pending);
        assert!(confirmation_ready(&db, &busy_ready_marker(&choice_id, 3, 42), 3.0).unwrap());
    } else {
        release_rejected(click, executor, server.clone()).await;
        assert!(queue::list(&db).unwrap().is_empty());
        assert!(!confirmation_ready(&db, &busy_ready_marker(&choice_id, 3, 42), 3.0).unwrap());
        assert_eq!(
            claims::get_busy_choice(&db, &choice_id, 0.0).unwrap(),
            Some(choice)
        );
        assert_eq!(
            ingress::get(&db, "interaction:201")
                .unwrap()
                .unwrap()
                .target_thread_id
                .as_deref(),
            Some("A")
        );
    }
    assert!(prompt_intake::list_prompt_intakes(&db).unwrap().is_empty());
    let saved = ingress::get(&db, &work.custody_ingress_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.payload, original.payload);
    assert_eq!(saved.target_thread_id, original.target_thread_id);
    server.close().await.unwrap();
    assert!(!app::rpc_log(&log).iter().any(|value| matches!(
        value["method"].as_str(),
        Some("thread/start" | "turn/start" | "thread/fork" | "thread/resume")
    )));
}

fn move_mapping(db: &std::path::Path) {
    mapping::upsert_thread(db, "A", "p", "a", 10, 43, 3.0).unwrap();
    mapping::upsert_thread(db, "B", "p", "b", 10, 42, 3.0).unwrap();
}

async fn activate(server: &cdr_app_server::ResidentAppServer) {
    server
        .execute(
            AppRequest {
                method: "test/active-turn",
                params: serde_json::json!({"threadId":"A","turnId":"current"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
}

async fn release_rejected(
    work: cdr_runtime::discord_dispatch::InboundInteractionWork,
    executor: Arc<ActionExecutor<AppServerTurnBackend>>,
    server: Arc<cdr_app_server::ResidentAppServer>,
) {
    let gate = http_gate::start().await;
    let http = Arc::new(
        twilight_http::Client::builder()
            .proxy(gate.address, true)
            .ratelimiter(None)
            .build(),
    );
    let (send, recv) = tokio::sync::mpsc::channel(1);
    send.send(work).await.unwrap();
    drop(send);
    let task = tokio::spawn(cdr_runtime::interaction_worker::run_interaction_worker(
        recv, executor, server, http,
    ));
    tokio::time::timeout(Duration::from_secs(3), gate.entered)
        .await
        .unwrap()
        .unwrap();
    gate.release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0]["content"]
            .as_str()
            .unwrap()
            .contains("original busy prompt route changed")
    );
}
