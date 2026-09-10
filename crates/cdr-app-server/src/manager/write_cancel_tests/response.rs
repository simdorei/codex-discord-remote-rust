use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use super::{server_from_client, test_client};
use crate::client::WriteTestPause;
use crate::{RequestId, ServerRequest, ServerRequestOccurrence};

#[tokio::test]
async fn resident_response_abort_after_bytes_quarantines_generation() {
    let pause = Arc::new(WriteTestPause::new());
    let client = test_client(Arc::clone(&pause));
    let occurrence = ServerRequestOccurrence::from_bytes(7_u128.to_be_bytes());
    let request = ServerRequest {
        id: RequestId::Integer(7),
        occurrence,
        method: "item/commandExecution/requestApproval".to_owned(),
        params: json!({"threadId": "thread-a", "command": "echo"}),
    };
    client
        .inner
        .state
        .lock()
        .expect("runtime state lock")
        .record_server_request(request.clone())
        .expect("record request");
    let server = Arc::new(server_from_client(client));
    let response = tokio::spawn({
        let server = Arc::clone(&server);
        async move { server.respond(&request.id, occurrence, json!({}), 1).await }
    });
    timeout(Duration::from_secs(2), pause.wait_until_before_lock())
        .await
        .expect("response passed initial check");
    timeout(Duration::from_secs(2), pause.wait_until_entered())
        .await
        .expect("response reached pre-flush pause");
    response.abort();
    assert!(response.await.expect_err("aborted response").is_cancelled());

    let snapshot = server.lifecycle_snapshot().await;
    let unsettled = server
        .has_unsettled_server_requests()
        .await
        .expect("resident state");
    timeout(Duration::from_secs(3), server.close())
        .await
        .expect("cleanup timeout")
        .expect("cleanup");

    assert!(snapshot.quarantined);
    assert!(snapshot.restart_pending);
    assert!(unsettled);
}
