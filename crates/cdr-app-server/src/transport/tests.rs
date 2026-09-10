use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, broadcast};

use super::handle_stdout_line;
use crate::RequestId;
use crate::client::{
    AppServerClient, ClientLifecycle, Inner, PendingOutcome, PendingResponse,
    insert_pending_response,
};
use crate::diagnostics::BoundedDiagnostics;
use crate::state::RuntimeState;

#[test]
fn response_delivery_releases_lifecycle_gate_after_pending_take() {
    let (notifications, _) = broadcast::channel(1);
    let (server_requests, _) = broadcast::channel(1);
    let client = AppServerClient {
        inner: Arc::new(Inner {
            child: AsyncMutex::new(None),
            closed: AtomicBool::new(false),
            diagnostics: Mutex::new(BoundedDiagnostics::default()),
            lifecycle: Arc::new(ClientLifecycle::new()),
            notifications,
            pending: Mutex::new(HashMap::new()),
            server_requests,
            state: Mutex::new(RuntimeState::starting(None)),
            stdin: AsyncMutex::new(None),
            write_pause: Mutex::new(None),
        }),
    };
    let id = RequestId::String("response-lock-order".to_owned());
    let permit = client.inner.lifecycle.admit().expect("transport lease");
    let (pending, mut receiver) = PendingResponse::new(permit);
    drop(insert_pending_response(&client.inner, id.clone(), pending));

    handle_stdout_line(
        &client.inner,
        &json!({"id": id, "result": {"delivered": true}}).to_string(),
    );

    assert!(matches!(
        receiver.try_recv(),
        Ok(PendingOutcome::Response(Ok(result)))
            if result == json!({"delivered": true})
    ));
    assert!(client.inner.lifecycle.seal_if_quiescent(|| true));
}
