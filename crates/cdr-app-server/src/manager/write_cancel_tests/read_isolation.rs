use super::*;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};
use tokio::task::JoinHandle;

const BARRIER: Duration = Duration::from_secs(2);
const EXPIRES: Duration = Duration::from_millis(100);

#[path = "read_isolation/recovery_observation.rs"]
mod recovery_observation;

struct Fixture {
    server: Arc<ResidentAppServer>,
    client: AppServerClient,
    wire: Lines<BufReader<DuplexStream>>,
    replies: DuplexStream,
    notices: broadcast::Receiver<crate::Notification>,
    reader: JoinHandle<()>,
}

impl Fixture {
    fn new() -> Self {
        let (stdin, wire) = tokio::io::duplex(65_536);
        let (replies, stdout) = tokio::io::duplex(65_536);
        let (notifications, _) = broadcast::channel(16);
        let (server_requests, _) = broadcast::channel(16);
        let mut state = RuntimeState::starting(None);
        state.initialized = true;
        state.generation = 1;
        let client = AppServerClient {
            inner: Arc::new(Inner {
                child: AsyncMutex::new(None),
                closed: AtomicBool::new(false),
                diagnostics: Mutex::new(BoundedDiagnostics::default()),
                lifecycle: Arc::new(ClientLifecycle::new()),
                notifications,
                pending: Mutex::new(HashMap::new()),
                server_requests,
                state: Mutex::new(state),
                stdin: AsyncMutex::new(Some(Box::pin(stdin))),
                write_pause: Mutex::new(None),
            }),
        };
        let reader = tokio::spawn(crate::transport::drain_stdout(
            Arc::clone(&client.inner),
            Box::pin(stdout),
        ));
        Self {
            server: Arc::new(server_from_client(client.clone())),
            notices: client.subscribe_notifications(),
            client,
            wire: BufReader::new(wire).lines(),
            replies,
            reader,
        }
    }

    fn request(
        &self,
        method: &'static str,
        wait: Duration,
    ) -> JoinHandle<Result<Value, AppServerError>> {
        let server = Arc::clone(&self.server);
        tokio::spawn(async move {
            let params = if method.starts_with("thread/") || method.starts_with("turn/") {
                json!({"threadId":"thread-a"})
            } else {
                json!({})
            };
            server.request(method, params, wait, Some(1)).await
        })
    }

    async fn next(&mut self) -> Value {
        let line = timeout(BARRIER, self.wire.next_line())
            .await
            .expect("actual pipe request deadline")
            .expect("pipe read")
            .expect("request line");
        serde_json::from_str(&line).expect("actual JSON request")
    }

    async fn send(&mut self, value: Value) {
        self.replies
            .write_all(format!("{value}\n").as_bytes())
            .await
            .expect("write reply");
        self.replies.flush().await.expect("flush reply");
    }

    async fn reply(&mut self, id: &Value, marker: &str) {
        self.send(json!({"id":id,"result":{"marker":marker}})).await;
    }

    async fn barrier(&mut self) {
        self.send(json!({"method":"p03a/barrier","params":{}}))
            .await;
        let notice = timeout(BARRIER, self.notices.recv())
            .await
            .expect("receiver barrier deadline")
            .expect("receiver barrier");
        assert_eq!(notice.method, "p03a/barrier");
    }

    fn pending(&self) -> usize {
        self.client.inner.pending.lock().expect("pending").len()
    }

    async fn wait_pending(&self, count: usize) {
        timeout(BARRIER, async {
            while self.pending() != count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("pending count deadline");
    }

    async fn deadlines_reclaimed(&self) {
        timeout(BARRIER, async {
            while Arc::weak_count(&self.client.inner) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("deadline ownership must be reclaimed, not retained until the old timeout");
    }

    async fn healthy_b(&mut self) {
        let b = self.request("model/list", BARRIER);
        let wire = self.next().await;
        assert_eq!(wire["method"], "model/list");
        self.reply(&wire["id"], "b").await;
        let value = timeout(BARRIER, b)
            .await
            .expect("B deadline")
            .expect("B task")
            .expect("B result");
        assert_eq!(value["marker"], "b");
        assert_eq!(self.pending(), 0);
        assert!(!self.server.lifecycle_snapshot().await.quarantined);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.reader.abort();
        crate::transport::mark_closed(&self.client.inner, "disposable read fixture complete");
    }
}

async fn local_timeout(method: &'static str) {
    let mut f = Fixture::new();
    let a = f.request(method, EXPIRES);
    assert_eq!(f.next().await["method"], method);
    let result = timeout(BARRIER, a)
        .await
        .expect("A deadline")
        .expect("A task");
    assert!(matches!(result, Err(AppServerError::Timeout { .. })));
    let snapshot = f.server.lifecycle_snapshot().await;
    assert!(
        !snapshot.quarantined && !snapshot.restart_pending,
        "{method} must fail locally"
    );
    f.healthy_b().await;
}

#[tokio::test]
async fn p03a_usage_timeout_is_local() {
    local_timeout("account/usage/read").await;
}

#[tokio::test]
async fn p03a_rate_limit_timeout_is_local() {
    local_timeout("account/rateLimits/read").await;
}

#[tokio::test]
async fn p03a_catalog_and_thread_read_timeouts_are_local() {
    for method in [
        "model/list",
        "thread/list",
        "thread/loaded/list",
        "thread/read",
        "thread/goal/get",
        "mcpServerStatus/list",
    ] {
        local_timeout(method).await;
    }
}

async fn late_a_while_b_waits(cancel: bool) {
    let mut f = Fixture::new();
    let a = f.request(
        "account/usage/read",
        if cancel {
            Duration::from_secs(30)
        } else {
            EXPIRES
        },
    );
    let old = f.next().await;
    drop(f.client.inner.stdin.lock().await); // Full writer/flush boundary, not just queued bytes.
    if cancel {
        a.abort();
        assert!(a.await.expect_err("A cancelled").is_cancelled());
        assert_eq!(
            f.pending(),
            0,
            "cancelled read must release response custody promptly"
        );
    } else {
        assert!(matches!(
            a.await.expect("A task"),
            Err(AppServerError::Timeout { .. })
        ));
    }
    assert!(
        !f.server.lifecycle_snapshot().await.quarantined,
        "clean A failure must admit B"
    );
    let b = f.request("model/list", BARRIER);
    let current = f.next().await;
    assert_ne!(old["id"], current["id"]);
    assert_eq!(
        f.pending(),
        1,
        "B is already waiting before A's late response"
    );
    f.reply(&old["id"], "late-a").await;
    f.reply(&old["id"], "duplicate-a").await;
    f.barrier().await;
    assert!(!b.is_finished(), "late A must not resolve waiting B");
    assert_eq!(f.pending(), 1);
    f.reply(&current["id"], "b").await;
    assert_eq!(
        timeout(BARRIER, b)
            .await
            .expect("B deadline")
            .expect("B task")
            .expect("B")["marker"],
        "b"
    );
    assert_eq!(f.pending(), 0);
    assert!(!f.server.lifecycle_snapshot().await.quarantined);
    f.deadlines_reclaimed().await;
}

#[tokio::test]
async fn p03a_flushed_read_cancel_and_late_a_preserve_waiting_b() {
    late_a_while_b_waits(true).await;
}

#[tokio::test]
async fn p03a_timeout_and_duplicate_late_a_preserve_waiting_b() {
    late_a_while_b_waits(false).await;
}

#[tokio::test]
async fn p03a_previous_instance_response_cannot_resolve_new_instance() {
    let mut old = Fixture::new();
    let a = old.request("account/usage/read", Duration::from_secs(30));
    let old_wire = old.next().await;
    a.abort();
    let _ = a.await;
    let mut new = Fixture::new();
    let b = new.request("model/list", BARRIER);
    let new_wire = new.next().await;
    assert_ne!(old.server.instance_id(), new.server.instance_id());
    new.reply(&old_wire["id"], "foreign-old-instance").await;
    new.barrier().await;
    assert!(!b.is_finished());
    assert_eq!(new.pending(), 1);
    new.reply(&new_wire["id"], "new-b").await;
    assert_eq!(b.await.expect("B task").expect("B")["marker"], "new-b");
    assert!(!new.server.lifecycle_snapshot().await.quarantined);
}

#[tokio::test]
async fn p03a_read_cancel_before_writer_reclaims_pending_without_quarantine() {
    let mut f = Fixture::new();
    let client = f.client.clone();
    let held = client.inner.stdin.lock().await;
    let a = f.request("account/usage/read", Duration::from_secs(30));
    f.wait_pending(1).await;
    a.abort();
    assert!(a.await.expect_err("cancelled").is_cancelled());
    assert_eq!(f.pending(), 0);
    assert!(!f.server.lifecycle_snapshot().await.quarantined);
    drop(held);
    f.healthy_b().await; // First observed wire request is B: A sent zero bytes.
    f.deadlines_reclaimed().await;
}

#[tokio::test]
async fn p03a_read_deadline_before_writer_sends_no_bytes_and_stays_healthy() {
    let mut f = Fixture::new();
    let client = f.client.clone();
    let held = client.inner.stdin.lock().await;
    let a = f.request("account/rateLimits/read", EXPIRES);
    f.wait_pending(1).await;
    f.wait_pending(0).await;
    drop(held);
    assert!(matches!(
        a.await.expect("A task"),
        Err(AppServerError::Timeout { .. })
    ));
    assert!(!f.server.lifecycle_snapshot().await.quarantined);
    f.healthy_b().await;
}

#[tokio::test]
async fn p03a_partial_read_cancel_still_seals_transport() {
    let pause = Arc::new(WriteTestPause::new());
    let server = Arc::new(test_server(Arc::clone(&pause)));
    let a = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .request(
                    "account/usage/read",
                    json!({}),
                    Duration::from_secs(30),
                    Some(1),
                )
                .await
        }
    });
    timeout(BARRIER, pause.wait_until_entered())
        .await
        .expect("partial write barrier");
    a.abort();
    assert!(a.await.expect_err("cancelled").is_cancelled());
    let state = server.lifecycle_snapshot().await;
    timeout(Duration::from_secs(3), server.close())
        .await
        .expect("close deadline")
        .expect("close");
    assert!(state.quarantined && state.restart_pending);
}

#[tokio::test]
async fn p03a_read_write_error_still_seals_transport() {
    let server = test_server(Arc::new(WriteTestPause::failing()));
    let result = server
        .request("model/list", json!({}), BARRIER, Some(1))
        .await;
    let state = server.lifecycle_snapshot().await;
    timeout(Duration::from_secs(3), server.close())
        .await
        .expect("close deadline")
        .expect("close");
    assert!(matches!(result, Err(AppServerError::Io(_))));
    assert!(state.quarantined && state.restart_pending);
}

#[tokio::test]
async fn p03a_mutation_and_unknown_timeouts_still_quarantine() {
    for method in [
        "thread/start",
        "thread/resume",
        "turn/start",
        "mcpServer/tool/call",
        "unknown/read",
    ] {
        let mut f = Fixture::new();
        let a = f.request(method, EXPIRES);
        assert_eq!(f.next().await["method"], method);
        assert!(matches!(
            a.await.expect("A task"),
            Err(AppServerError::Timeout { .. })
        ));
        let state = f.server.lifecycle_snapshot().await;
        assert!(
            state.quarantined && state.restart_pending,
            "{method} is not an observational request"
        );
    }
}

#[tokio::test]
async fn p03a_cancelled_mutation_retains_its_response_lease() {
    let mut f = Fixture::new();
    let a = f.request("turn/start", Duration::from_secs(30));
    let wire = f.next().await;
    drop(f.client.inner.stdin.lock().await);
    a.abort();
    assert!(a.await.expect_err("cancelled").is_cancelled());
    assert_eq!(
        f.pending(),
        1,
        "do not shorten unresolved mutation response custody"
    );
    assert!(!f.client.seal_if_quiescent());
    assert!(f.server.lifecycle_snapshot().await.quarantined);
    f.reply(&wire["id"], "late-mutation").await;
    f.barrier().await;
    assert_eq!(f.pending(), 0);
}

#[tokio::test]
async fn p03a_pending_capacity_refuses_without_eviction_or_write() {
    let f = Fixture::new();
    let client = f.client.clone();
    let held = client.inner.stdin.lock().await;
    let tasks: Vec<_> = (0..1_024)
        .map(|_| f.request("model/list", Duration::from_secs(30)))
        .collect();
    f.wait_pending(1_024).await;
    let mut excess = f.request("model/list", Duration::from_secs(30));
    let result = timeout(Duration::from_secs(1), &mut excess).await;
    let refused = matches!(&result, Ok(Ok(Err(error))) if error.to_string().contains("pending response capacity"));
    let retained = f.pending();
    if result.is_err() {
        excess.abort();
        let _ = excess.await;
    }
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }
    drop(held);
    assert!(
        refused,
        "capacity must refuse before the blocked writer, not wait or evict"
    );
    assert_eq!(retained, 1_024);
    assert_eq!(f.pending(), 0);
    assert!(!f.server.lifecycle_snapshot().await.quarantined);
    f.deadlines_reclaimed().await;
}

#[tokio::test]
async fn p03a_completed_requests_reclaim_deadline_tasks() {
    let mut f = Fixture::new();
    for _ in 0..64 {
        let a = f.request("model/list", Duration::from_secs(30));
        let wire = f.next().await;
        f.reply(&wire["id"], "complete").await;
        assert_eq!(a.await.expect("A task").expect("A")["marker"], "complete");
    }
    assert_eq!(f.pending(), 0);
    f.deadlines_reclaimed().await;
}

#[tokio::test]
async fn p03a_repeated_expiry_and_late_duplicates_keep_retained_state_bounded() {
    let mut f = Fixture::new();
    let mut last_id = Value::Null;
    for _ in 0..16 {
        let a = f.request("thread/read", Duration::from_millis(10));
        last_id = f.next().await["id"].clone();
        assert!(matches!(
            a.await.expect("A task"),
            Err(AppServerError::Timeout { .. })
        ));
        assert_eq!(f.pending(), 0);
    }
    for _ in 0..1_024 {
        f.reply(&last_id, "late-duplicate").await;
    }
    f.barrier().await;
    let diagnostics = f.client.diagnostic_snapshot();
    assert!(diagnostics.lines.len() <= 512);
    assert!(diagnostics.retained_bytes <= 65_536);
    assert!(diagnostics.dropped_lines > 0);
    assert_eq!(f.pending(), 0);
    f.deadlines_reclaimed().await;
    f.healthy_b().await;
}
