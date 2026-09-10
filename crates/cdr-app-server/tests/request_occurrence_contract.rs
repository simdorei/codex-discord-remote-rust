use std::time::Duration;

use cdr_app_server::{
    AppServerClient, AppServerConfig, AppServerError, RequestId, RpcErrorPayload,
    ServerRequestOccurrence,
};
use serde_json::json;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(1);

fn fake_config() -> AppServerConfig {
    let mut config = AppServerConfig::new(env!("CARGO_BIN_EXE_fake_codex_app_server"));
    config.arguments.clear();
    config
}

async fn request_approval(client: &AppServerClient) -> cdr_app_server::ServerRequest {
    let mut requests = client.subscribe_server_requests();
    client
        .request("test/requestApproval", json!({}), WAIT)
        .await
        .expect("request approval");
    requests.recv().await.expect("approval request")
}

#[test]
fn occurrence_is_an_opaque_exact_128_bit_value() {
    let bytes = [0x5a; 16];
    let occurrence = ServerRequestOccurrence::from_bytes(bytes);
    assert_eq!(occurrence.as_bytes(), &bytes);
    assert_eq!(occurrence, occurrence);
}

#[tokio::test]
async fn identical_pending_redelivery_reuses_occurrence_without_order_alias() {
    let client = AppServerClient::start(fake_config()).await.expect("start");
    let mut requests = client.subscribe_server_requests();
    client
        .request("test/requestApprovalDuplicate", json!({}), WAIT)
        .await
        .expect("request duplicate");

    let first = requests.recv().await.expect("first delivery");
    assert!(
        timeout(Duration::from_millis(30), requests.recv())
            .await
            .is_err()
    );

    let first_snapshot = client.pending_server_requests(Some("thread-a"));
    let second_snapshot = client.pending_server_requests(Some("thread-a"));
    assert_eq!(first_snapshot.len(), 1);
    assert_eq!(second_snapshot, first_snapshot);
    assert_eq!(first_snapshot[0].occurrence, first.occurrence);
    client.close().await.expect("close");
}

#[tokio::test]
async fn conflicting_pending_redelivery_is_rejected_without_overwrite() {
    let client = AppServerClient::start(fake_config()).await.expect("start");
    let mut requests = client.subscribe_server_requests();
    client
        .request("test/requestApprovalConflict", json!({}), WAIT)
        .await
        .expect("request conflict");

    let first = requests.recv().await.expect("first delivery");
    assert!(
        timeout(Duration::from_millis(30), requests.recv())
            .await
            .is_err()
    );
    let pending = client.pending_server_requests(Some("thread-a"));
    assert_eq!(pending, vec![first]);
    assert_eq!(pending[0].params["command"], "echo safe");
    assert!(
        client
            .diagnostic_snapshot()
            .lines
            .iter()
            .any(|line| line.contains("conflicting pending server request"))
    );
    client.close().await.expect("close");
}

#[tokio::test]
async fn resolved_id_reuse_gets_new_occurrence_and_rejects_stale_response() {
    let client = AppServerClient::start(fake_config()).await.expect("start");
    let first = request_approval(&client).await;
    client
        .respond(&first.id, first.occurrence, json!({"decision": "accept"}))
        .await
        .expect("respond to first");

    let second = request_approval(&client).await;
    assert_eq!(second.id, first.id);
    assert_ne!(second.occurrence, first.occurrence);
    let stale = client
        .respond(&first.id, first.occurrence, json!({"decision": "decline"}))
        .await
        .expect_err("stale response must fail before write");
    assert!(matches!(stale, AppServerError::StaleServerRequest { .. }));
    assert_eq!(client.pending_server_requests(None), vec![second.clone()]);

    client
        .respond_error(
            &second.id,
            second.occurrence,
            RpcErrorPayload {
                code: -32_800,
                message: "cancelled".to_owned(),
                data: None,
            },
        )
        .await
        .expect("respond to current occurrence");
    assert!(client.pending_server_requests(None).is_empty());
    client.close().await.expect("close");
}

#[tokio::test]
async fn fresh_process_assigns_new_occurrence_to_same_wire_id() {
    let first_client = AppServerClient::start(fake_config())
        .await
        .expect("start first");
    let first = request_approval(&first_client).await;
    first_client.close().await.expect("close first");

    let second_client = AppServerClient::start(fake_config())
        .await
        .expect("start second");
    let second = request_approval(&second_client).await;
    assert_eq!(second.id, first.id);
    assert_ne!(second.occurrence, first.occurrence);
    assert_eq!(first.occurrence.as_bytes()[6] >> 4, 4, "UUID version");
    assert_eq!(first.occurrence.as_bytes()[8] & 0xc0, 0x80, "UUID variant");
    assert_eq!(second.occurrence.as_bytes()[6] >> 4, 4, "UUID version");
    assert_eq!(second.occurrence.as_bytes()[8] & 0xc0, 0x80, "UUID variant");
    second_client.close().await.expect("close second");
}

#[tokio::test]
async fn string_and_integer_ids_remain_distinct_and_cancel_exact_occurrences() {
    let client = AppServerClient::start(fake_config()).await.expect("start");
    let mut requests = client.subscribe_server_requests();
    client
        .request("test/requestTypedIds", json!({}), WAIT)
        .await
        .expect("request typed ids");
    let string_request = requests.recv().await.expect("string request");
    let integer_request = requests.recv().await.expect("integer request");
    assert_eq!(string_request.id, RequestId::String("1".to_owned()));
    assert_eq!(integer_request.id, RequestId::Integer(1));
    assert_ne!(string_request.occurrence, integer_request.occurrence);
    assert_eq!(client.pending_server_requests(Some("thread-a")).len(), 2);

    assert_eq!(
        client
            .cancel_pending_server_requests("thread-a")
            .await
            .expect("cancel exact occurrences"),
        2
    );
    assert!(client.pending_server_requests(Some("thread-a")).is_empty());
    client.close().await.expect("close");
}
