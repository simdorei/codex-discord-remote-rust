use cdr_runtime::{bridge_state::BridgeState, component_worker::handle_component_work};
use cdr_store::queue;
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/approval_app_server.rs"]
mod app;
#[path = "support/approval_click.rs"]
mod click;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn bound_approval_checks_original_actor_room_turn_and_confirmation_identity() {
    for (channel, user, expired, allowed) in [
        (42, 4, false, false),
        (43, 3, false, false),
        (42, 3, true, false),
        (42, 3, false, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log).await);
        let db = temp.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0)
            .unwrap();
        let executor = target::executor(
            &temp,
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            Arc::new(target::FakeBackend::default()),
        )
        .with_server(server.clone());
        queue::enqueue(
            &db,
            queue::NewQueueJob {
                job_id: "active-job",
                target_thread_id: "thread-b",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(101),
                app_server_generation: i64::try_from(server.generation()).unwrap(),
                prompt: "original",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        cdr_store::schema::open_initialized(&db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET state='running',turn_id='turn-b',attempt_count=1",
                [],
            )
            .unwrap();
        server
            .request("test/pending", json!({}), Duration::from_secs(2), None)
            .await
            .unwrap();
        let pending = server.pending_server_requests(None).await.unwrap();
        let prompt =
            cdr_runtime::server_prompt::build_server_prompt(&pending[0], server.generation())
                .unwrap();
        let row = serde_json::to_value(&prompt.components[0]).unwrap();
        let component = cdr_discord::components::parse_component_id(
            row["components"][0]["custom_id"].as_str().unwrap(),
        )
        .unwrap();
        let work = click::click(
            &db,
            channel,
            user,
            row["components"][0]["custom_id"].as_str().unwrap(),
        )
        .await;
        if expired {
            server
                .request("test/finish", json!({}), Duration::from_secs(2), None)
                .await
                .unwrap();
        }
        let result = handle_component_work(&work, &component, &executor, &server).await;
        let after = server.pending_server_requests(None).await.unwrap();
        if allowed {
            assert!(result.is_ok(), "original sender must be able to answer");
            assert!(after.is_empty());
            assert!(
                handle_component_work(&work, &component, &executor, &server)
                    .await
                    .is_ok(),
                "same actor may retrieve existing confirmation only"
            );
            let mut foreign = work.clone();
            foreign.user_id = twilight_model::id::Id::new(4);
            assert!(
                handle_component_work(&foreign, &component, &executor, &server)
                    .await
                    .is_err(),
                "confirmation cannot transfer to another user"
            );
        } else {
            assert!(
                result.is_err(),
                "another actor/room or ended turn answered the original request"
            );
            assert_eq!(pending, after);
        }
        server.close().await.unwrap();
        assert_response_count(&log, allowed);
    }
}

fn assert_response_count(log: &std::path::Path, allowed: bool) {
    let calls = std::fs::read_to_string(log).unwrap();
    let responses = calls
        .lines()
        .filter(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            value["id"] == "approval-1" && value.get("result").is_some()
        })
        .count();
    assert_eq!(responses, usize::from(allowed));
}
