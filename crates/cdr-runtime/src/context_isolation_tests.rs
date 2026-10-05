//! S3-01 characterization: a late blocking read must not multiply or block control.
use super::{READER, bounded_reader, render};
use crate::test_support::{approval_http, message_fixture::MessageFixture};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const CHILD: &str = "context_view::isolation_tests::context_reader_isolation_child";

#[test]
fn context_reader_isolation_uses_a_separate_native_process() {
    let root = tempfile::tempdir().unwrap();
    let output_path = root.path().join("child.log");
    let output = std::fs::File::create(&output_path).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", CHILD, "--nocapture"])
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!(
                "context isolation child exceeded 30s: {}",
                std::fs::read_to_string(&output_path).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let log = std::fs::read_to_string(output_path).unwrap();
    println!("{log}");
    assert!(status.success(), "context isolation child failed: {status}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "executed by the isolated native-process parent"]
async fn context_reader_isolation_child() {
    let root = tempfile::tempdir().unwrap();
    let http = approval_http::start().await;
    let client = Arc::new(
        twilight_http::Client::builder()
            .proxy(http.address, true)
            .ratelimiter(None)
            .build(),
    );
    let fixture = control_fixture(&root, client).await;
    crate::idle_release::install(&fixture.server, fixture.executor.mirror_db()).unwrap();
    install_rollout(&root);
    seed_original_control(&fixture);
    fixture
        .server
        .execute(
            cdr_app_server::requests::AppRequest {
                method: "thread/read",
                params: serde_json::json!({"threadId":"thread-b"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();

    let reads = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&reads);
    let (release, held) = std::sync::mpsc::channel();
    let (started, notification) = tokio::sync::oneshot::channel();
    let before = Instant::now();
    let worker = tokio::spawn(bounded_reader(&READER, Duration::from_secs(3), move || {
        observed.fetch_add(1, Ordering::SeqCst);
        started.send(()).unwrap();
        held.recv_timeout(Duration::from_secs(15)).unwrap();
        "late snapshot must not be delivered".into()
    }));
    notification.await.unwrap();
    concurrent_queries_are_busy().await;
    let result = tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        result.unwrap_err(),
        "context read timed out; snapshot unavailable, OS read may still be finishing"
    );
    assert!(before.elapsed() >= Duration::from_secs(3));
    println!(
        "context_isolation stage=timeout elapsed_ms={}",
        before.elapsed().as_millis()
    );
    concurrent_queries_are_busy().await;
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(READER.available_permits(), 0);

    commands_while_blocked(&fixture, root.path()).await;

    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while READER.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        crate::message_worker::process_admitted_gateway_message(
            fixture.admit_id("!context refresh 5", 903),
            &fixture.context(root.path()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    http.stop.send(()).unwrap();
    let traffic = http.task.await.unwrap();
    assert_traffic(&traffic);
    fixture.server.close().await.unwrap();
    assert_exact_control_only(root.path());
}

fn assert_traffic(traffic: &[(bool, serde_json::Value)]) {
    assert_eq!(traffic.len(), 3);
    eprintln!("context_isolation stop_response={}", traffic[1].1);
    let status = traffic[0].1["content"].as_str().unwrap();
    assert!(status.contains("Codex thread status") && status.contains("thread_id: thread-b"));
    assert!(status.contains("context reader is still running; no extra reader started"));
    assert!(
        traffic[1].1["content"]
            .as_str()
            .unwrap()
            .contains("Stop accepted for thread-b")
    );
    assert!(
        traffic[1].1["content"]
            .as_str()
            .unwrap()
            .contains("operation_id:")
    );
    let context = traffic[2].1["content"].as_str().unwrap();
    assert!(context.contains("last_input: 12345") && context.contains("fresh after blocked read"));
    assert!(
        !traffic
            .iter()
            .any(|(_, body)| body.to_string().contains("late snapshot"))
    );
}

async fn control_fixture(
    root: &tempfile::TempDir,
    client: Arc<twilight_http::Client>,
) -> MessageFixture {
    let db = root.path().join("mirror.sqlite");
    let fence = Arc::new(
        crate::dead_generation_recovery::RuntimeDeadGenerationFence::new(
            db,
            "context-fixture".into(),
            None,
        )
        .unwrap(),
    );
    let active = root.path().join("next-active.json");
    std::fs::write(
        &active,
        serde_json::to_vec(&serde_json::json!({"threadId":"thread-b","turnId":"original-control"}))
            .unwrap(),
    )
    .unwrap();
    let mut config = crate::soak::native_fixture::config("action");
    config.environment.insert(
        "CDR_STOP_NEXT_ACTIVE_PATH".into(),
        active.to_string_lossy().into_owned(),
    );
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        root.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(
        cdr_app_server::ResidentAppServer::start_with_dead_generation_fence(config, fence)
            .await
            .unwrap(),
    );
    MessageFixture::with_server(root, client, server)
}

fn seed_original_control(fixture: &MessageFixture) {
    let db = fixture.executor.mirror_db();
    let generation = i64::try_from(fixture.server.generation()).unwrap();
    cdr_store::queue::enqueue(
        db,
        cdr_store::queue::NewQueueJob {
            job_id: "context-original",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "original context fixture request",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(db, "context-original", &[], generation).unwrap();
    cdr_store::queue::mark_running(db, "context-original", "original-control", generation).unwrap();
}

async fn commands_while_blocked(fixture: &MessageFixture, root: &std::path::Path) {
    for (command, id) in [("!status", 901), ("!stop", 902)] {
        let before = Instant::now();
        tokio::time::timeout(
            Duration::from_secs(3),
            crate::message_worker::process_admitted_gateway_message(
                fixture.admit_id(command, id),
                &fixture.context(root),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(READER.available_permits(), 0);
        println!(
            "context_isolation command={command} elapsed_ms={}",
            before.elapsed().as_millis()
        );
    }
    let before = Instant::now();
    let mut cursor = 0;
    let count = tokio::time::timeout(
        Duration::from_secs(3),
        fixture.executor.process_stop_controls(&mut cursor),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(READER.available_permits(), 0);
    println!(
        "context_isolation stage=interrupt elapsed_ms={}",
        before.elapsed().as_millis()
    );
}

async fn concurrent_queries_are_busy() {
    let queries = (0..16)
        .map(|_| tokio::spawn(render(Vec::new(), false, 5)))
        .collect::<Vec<_>>();
    for query in queries {
        assert_eq!(
            query.await.unwrap().unwrap_err(),
            "context reader is still running; no extra reader started"
        );
    }
}

fn install_rollout(root: &tempfile::TempDir) {
    let path = root.path().join("context.jsonl");
    std::fs::write(&path, concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-b\"}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":12345}}}}\n",
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final\",\"content\":[{\"type\":\"output_text\",\"text\":\"fresh after blocked read\"}]}}\n"
    )).unwrap();
    rusqlite::Connection::open(root.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "UPDATE threads SET rollout_path=? WHERE id='thread-b'",
            [path.to_string_lossy().as_ref()],
        )
        .unwrap();
}

fn assert_exact_control_only(root: &std::path::Path) {
    let rpc = std::fs::read_to_string(root.join("rpc.jsonl")).unwrap();
    let calls = rpc
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let stops = calls
        .iter()
        .filter(|call| call["method"] == "turn/interrupt")
        .collect::<Vec<_>>();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["params"]["threadId"], "thread-b");
    assert_eq!(stops[0]["params"]["turnId"], "original-control");
    assert!(!calls.iter().any(|call| matches!(
        call["method"].as_str(),
        Some("turn/start" | "turn/steer" | "thread/resume" | "thread/fork" | "thread/unsubscribe")
    )));
}
