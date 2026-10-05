use crate::message_worker::{
    ErrorReportTarget, admit_message_candidate_at, process_admitted_gateway_message,
    process_with_error_report, report_processing_error,
};
use crate::test_support::{message_fixture::MessageFixture, native_fixture};
use cdr_discord::http::DiscordHttp;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use twilight_model::id::Id;

const TEST_NAME: &str = "writer_recovery::admitted_handoff_tests::admitted_recovery_has_one_handoff_budget_without_a_live_app_server";

#[tokio::test]
async fn admitted_recovery_has_one_handoff_budget_without_a_live_app_server() {
    if std::env::var("CDR_ADMITTED_RECOVERY_FIXTURE").as_deref() == Ok("child") {
        child_flow().await;
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("scripts")).unwrap();
    std::fs::write(
        temp.path().join("scripts/Invoke-CdrDesktopRecovery.ps1"),
        CONTROLLER,
    )
    .unwrap();
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
        .env("CDR_ADMITTED_RECOVERY_FIXTURE", "child")
        .current_dir(temp.path())
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn child_flow() {
    let root = std::env::current_dir().unwrap();
    let temp = tempfile::TempDir::new_in(&root).unwrap();
    let http = HttpFixture::start().await;
    let mut config = native_fixture::config("repair");
    config.environment.insert(
        "CDR_REPAIR_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into(),
    );
    config
        .environment
        .insert("CDR_REPAIR_MODE".into(), "ready".into());
    let server = Arc::new(
        cdr_app_server::ResidentAppServer::start(config)
            .await
            .unwrap(),
    );
    let fixture = MessageFixture::with_server(&temp, http.client.clone(), server.clone());
    // Only our isolated fixture server is closed. Recovery must not need a live terminal/RPC.
    server.close().await.unwrap();
    let database = fixture.executor.mirror_db();
    cdr_store::queue::enqueue(
        database,
        cdr_store::queue::NewQueueJob {
            job_id: "recover-original",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(71),
            app_server_generation: 1,
            prompt: "do not replay this original",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    let started = Instant::now();
    let admitted = fixture.admit("!recover");
    let context = fixture.context(temp.path());
    let api = DiscordHttp::new(http.client.clone(), Id::new(1));
    let target = ErrorReportTarget {
        channel_id: Id::new(42),
        message_id: Id::new(801),
    };
    process_with_error_report(
        target,
        admitted,
        |work| process_admitted_gateway_message(work, &context),
        |target, error| report_processing_error(database, &api, target, error),
    )
    .await
    .unwrap();
    let elapsed = started.elapsed();
    let replies = http.messages.lock().unwrap().clone();
    assert_cancelled_original(&root, database, elapsed, &replies);
    let operation = root
        .join("maintenance_backups/desktop-recovery/abcdef0123456789abcdef0123456789/receipt.json");
    let saved = std::fs::read(&operation).unwrap();
    let receipt: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(receipt["Phase"], "dispatched");
    assert_eq!(receipt["ThreadId"], "thread-b");
    let duplicate = fixture.classify_id("!recover", 801);
    assert!(
        admit_message_candidate_at(duplicate, SystemTime::now())
            .unwrap()
            .is_none()
    );
    assert_eq!(std::fs::read(operation).unwrap(), saved);
    assert_eq!(
        std::fs::read_to_string(root.join("fixture-dispatches"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(
        elapsed < Duration::from_secs(22),
        "sequential controller waits exceeded the fixed total handoff oracle: {elapsed:?}"
    );
    assert_eq!(replies.len(), 1);
    assert!(
        replies[0]["content"]
            .as_str()
            .unwrap()
            .contains("recovery handoff timed out"),
        "{replies:?}"
    );
    let rpc = std::fs::read_to_string(temp.path().join("rpc.jsonl")).unwrap();
    assert_eq!(
        rpc.lines().count(),
        1,
        "recovery waited for or mutated the closed app-server"
    );
}

fn assert_cancelled_original(
    root: &std::path::Path,
    database: &std::path::Path,
    elapsed: Duration,
    replies: &[Value],
) {
    let dispatches = std::fs::read_to_string(root.join("fixture-dispatches"))
        .unwrap()
        .lines()
        .count();
    let cancelled: i64 = cdr_store::schema::open_initialized(database)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM codex_request_cancellations WHERE job_id='recover-original'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    eprintln!(
        "admitted_handoff elapsed_ms={} cancellations={cancelled} dispatches={dispatches} replies={replies:?}",
        elapsed.as_millis()
    );
    assert_eq!(cancelled, 1);
    assert_eq!(dispatches, 1);
    assert!(cdr_store::queue::list(database).unwrap().is_empty());
    assert!(
        cdr_store::queue::try_begin_attempt(database, "recover-original", &[], 1)
            .unwrap()
            .is_none()
    );
    assert!(
        cdr_store::queue::enqueue(
            database,
            cdr_store::queue::NewQueueJob {
                job_id: "must-not-replay",
                target_thread_id: "thread-b",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(71),
                app_server_generation: 2,
                prompt: "forbidden replay",
                queued: true,
                ack_sent: true,
                created_at: 2.0,
            }
        )
        .is_err()
    );
}

struct HttpFixture {
    client: Arc<twilight_http::Client>,
    messages: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl HttpFixture {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let messages = Arc::new(Mutex::new(Vec::new()));
        let seen = messages.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                let content = request["content"].as_str().unwrap_or("").to_owned();
                seen.lock().unwrap().push(request);
                let body = json!({
                    "id":"9901","channel_id":"42","author":{"id":"1","username":"fixture","discriminator":"0001","avatar":null,"bot":true},
                    "content":content,"timestamp":"2020-02-02T02:02:02.020000+00:00","edited_timestamp":null,
                    "tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],
                    "attachments":[],"embeds":[],"pinned":false,"type":0
                }).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = Arc::new(
            twilight_http::Client::builder()
                .token("isolated-fixture".into())
                .proxy(address.to_string(), true)
                .build(),
        );
        Self {
            client,
            messages,
            task,
        }
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let end = loop {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 32_768);
        if let Some(at) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    let length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(length < 32_768);
    while bytes.len() < end + length {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&bytes[end..end + length]).unwrap()
}

const CONTROLLER: &str = r"
param([string]$Mode,[string]$RepoRoot,[string]$CodexHome,[string]$ThreadId,[string]$ExpectedRecoveryIdentity)
$ErrorActionPreference='Stop'
if($Mode -ceq 'InspectTools') {
    Start-Sleep -Seconds 12
    @{ThreadId=$ThreadId;State='tools_recovery';RecoveryIdentity='desktop;bot;writer'} | ConvertTo-Json -Compress
    exit 0
}
if($Mode -cne 'StartTools' -or $ExpectedRecoveryIdentity -cne 'desktop;bot;writer') { throw 'Unexpected fixture dispatch' }
[IO.File]::AppendAllText((Join-Path $RepoRoot 'fixture-dispatches'),('dispatch'+[Environment]::NewLine))
$operation='abcdef0123456789abcdef0123456789'
$bundle=Join-Path $RepoRoot ('maintenance_backups/desktop-recovery/'+$operation)
[void][IO.Directory]::CreateDirectory($bundle)
$receipt=Join-Path $bundle 'receipt.json'
[IO.File]::WriteAllText($receipt,(@{Phase='dispatched';Operation=$operation;ThreadId=$ThreadId} | ConvertTo-Json))
Start-Sleep -Seconds 12
@{Operation=$operation;ReceiptPath=$receipt;WorkerPid=4242} | ConvertTo-Json -Compress
";
