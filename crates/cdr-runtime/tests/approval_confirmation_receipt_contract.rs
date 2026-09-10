use cdr_runtime::component_worker::deliver_confirmation_and_clear;
use cdr_runtime::component_worker::{ConfirmationPlan, standard_confirmation_plan};
use std::sync::Arc;
use twilight_model::id::Id;
#[path = "../src/completion_worker/goal_mirror_http.rs"]
mod http_gate;

#[tokio::test]
async fn repeated_component_confirmation_uses_the_recorded_discord_message() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("receipt.sqlite");
    let gate = http_gate::start().await;
    let http = Arc::new(
        twilight_http::Client::builder()
            .proxy(gate.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    let component = cdr_discord::components::ComponentId::BoundApproval {
        thread_fingerprint: "0123456789abcdef".into(),
        request_fingerprint: "0123456789abcdef0123456789abcdef".into(),
        answer: cdr_discord::components::ApprovalAnswer::Approve,
    };
    let plan: ConfirmationPlan =
        standard_confirmation_plan(&component, "original-actor-claim").unwrap();
    gate.release.send(()).unwrap();
    deliver_confirmation_and_clear(http.clone(), &db, Id::new(42), None, &plan)
        .await
        .unwrap();
    gate.entered.await.unwrap();
    deliver_confirmation_and_clear(http, &db, Id::new(42), None, &plan)
        .await
        .unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(
        posts.len(),
        1,
        "a saved success confirmation must not create another message"
    );
}
