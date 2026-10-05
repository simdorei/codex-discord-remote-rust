#[path = "support/session_mirror_background_safety.rs"]
mod safety;

use std::{
    fs::{self, OpenOptions},
    future::Future,
    io::Write,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use cdr_runtime::session_mirror::{
    DiscordSessionMirrorSender, SessionMirrorDeliveryIdentity, SessionMirrorSender,
    run_session_mirror_worker,
};
use cdr_runtime::session_mirror_worker::SessionMirrorWorker;
use cdr_store::{
    mapping::upsert_thread,
    mirror::{get_offset, update_cursor},
};
use rusqlite::{Connection, params};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, watch},
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use twilight_http::Client;

const HEALTHY_B_BUDGET: Duration = Duration::from_secs(5);

struct Fixture {
    temp: tempfile::TempDir,
    state_db: PathBuf,
    mirror_db: PathBuf,
    b_rollout: PathBuf,
}

impl Fixture {
    fn new(b_has_message: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state_db = temp.path().join("state.sqlite");
        let mirror_db = temp.path().join("mirror.sqlite");
        let a_rollout = temp.path().join("a.jsonl");
        let b_rollout = temp.path().join("b.jsonl");
        fs::write(&a_rollout, event("slow A")).unwrap();
        fs::write(
            &b_rollout,
            if b_has_message {
                event("healthy B first")
            } else {
                String::new()
            },
        )
        .unwrap();
        let connection = Connection::open(&state_db).unwrap();
        connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, thread_source TEXT);").unwrap();
        for (id, title, updated, channel, rollout) in [
            ("thread-a", "A", 2_i64, 201, &a_rollout),
            ("thread-b", "B", 1_i64, 202, &b_rollout),
        ] {
            connection.execute(
                "INSERT INTO threads VALUES (?1,?2,'C:/repo',?3,?4,'gpt','high',0,0,0,'vscode','user')",
                params![id, title, updated, rollout.to_string_lossy().as_ref()],
            ).unwrap();
            upsert_thread(
                &mirror_db,
                id,
                "project",
                title,
                100,
                channel,
                if id == "thread-a" { 2.0 } else { 1.0 },
            )
            .unwrap();
            update_cursor(&mirror_db, id, rollout.to_string_lossy().as_ref(), 0, 1.0).unwrap();
        }
        Self {
            temp,
            state_db,
            mirror_db,
            b_rollout,
        }
    }

    fn append_b(&self, text: &str) {
        OpenOptions::new()
            .append(true)
            .open(&self.b_rollout)
            .unwrap()
            .write_all(event(text).as_bytes())
            .unwrap();
    }

    async fn b_committed(&self) {
        let expected = i64::try_from(fs::metadata(&self.b_rollout).unwrap().len()).unwrap();
        loop {
            if get_offset(&self.mirror_db, "thread-b")
                .unwrap()
                .unwrap()
                .cursor
                == expected
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn run<S: SessionMirrorSender + 'static>(&self, sender: Arc<S>) -> Running {
        let worker =
            SessionMirrorWorker::new(self.state_db.clone(), self.mirror_db.clone(), sender);
        let (shutdown, receiver) = watch::channel(false);
        Running {
            shutdown,
            task: tokio::spawn(run_session_mirror_worker(worker, receiver)),
        }
    }

    fn run_http(&self, server: &Server) -> Running {
        self.run(Arc::new(DiscordSessionMirrorSender::new(
            server.client(),
            self.mirror_db.clone(),
        )))
    }
}

fn event(text: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "timestamp":"2026-09-29T00:00:00Z", "type":"event_msg",
            "payload":{"type":"agent_message", "message":text}
        })
    )
}

struct Running {
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Running {
    async fn stop(mut self) {
        self.shutdown.send(true).unwrap();
        timeout(Duration::from_millis(250), &mut self.task)
            .await
            .expect("shutdown must cancel the in-flight mirror futures")
            .unwrap();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<(u64, String)>>>,
    a_started: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Server {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let a_started = Arc::new(Notify::new());
        let seen = requests.clone();
        let started = a_started.clone();
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        tasks.spawn(serve(socket, seen.clone(), started.clone()));
                    }
                    Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                        result.unwrap();
                    }
                }
            }
        });
        Self {
            address,
            requests,
            a_started,
            task,
        }
    }

    fn client(&self) -> Arc<Client> {
        Arc::new(
            Client::builder()
                .token("offline-fixture".to_owned())
                .proxy(self.address.to_string(), true)
                .build(),
        )
    }

    fn count(&self, channel: u64) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| *id == channel)
            .count()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn request(socket: &mut TcpStream) -> (u64, serde_json::Value) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let end = loop {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 32_768);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let channel = headers
        .lines()
        .next()
        .unwrap()
        .split("/channels/")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    assert!(length < 16_384);
    while bytes.len() < end + length {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    (
        channel,
        serde_json::from_slice(&bytes[end..end + length]).unwrap(),
    )
}

async fn serve(
    mut socket: TcpStream,
    seen: Arc<Mutex<Vec<(u64, String)>>>,
    a_started: Arc<Notify>,
) {
    let (channel, body) = request(&mut socket).await;
    let number = {
        let mut requests = seen.lock().unwrap();
        requests.push((channel, body["content"].as_str().unwrap().to_owned()));
        requests.len()
    };
    if channel == 201 {
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n{").await.unwrap();
        a_started.notify_one();
        tokio::time::sleep(Duration::from_secs(30)).await;
        return;
    }
    let response = serde_json::json!({
        "id":(9000 + number).to_string(), "channel_id":channel.to_string(),
        "author":{"id":"9","username":"fixture","discriminator":"0001","avatar":null,"bot":true},
        "content":body["content"], "timestamp":"2026-09-29T00:00:00+00:00",
        "edited_timestamp":null, "tts":false, "mention_everyone":false, "mentions":[],
        "mention_roles":[], "attachments":[], "embeds":[], "pinned":false, "type":0,
        "nonce":body["nonce"], "flags":0
    })
    .to_string();
    socket.write_all(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len(),
    ).as_bytes()).await.unwrap();
}

#[tokio::test]
async fn background_stalled_http_a_does_not_block_b_commit_or_cold_receipts() {
    let fixture = Fixture::new(true);
    let server = Server::new().await;
    let running = fixture.run_http(&server);
    timeout(HEALTHY_B_BUDGET, server.a_started.notified())
        .await
        .unwrap();
    timeout(HEALTHY_B_BUDGET, fixture.b_committed())
        .await
        .expect("healthy B receipt and cursor must commit within 5s while A stalls for 30s");
    assert_eq!(server.count(201), 1);
    assert_eq!(server.count(202), 1);
    fixture.append_b("healthy B later");
    timeout(HEALTHY_B_BUDGET, fixture.b_committed())
        .await
        .expect("new B events must be discovered without waiting for the old A batch");
    running.stop().await;

    let cold = fixture.run_http(&server);
    fixture.append_b("healthy B after cold restart");
    timeout(HEALTHY_B_BUDGET, fixture.b_committed())
        .await
        .expect("an unknown A receipt must not stop B after cold worker reconstruction");
    cold.stop().await;
    assert_eq!(
        server.count(201),
        1,
        "the accepted/unconfirmed A POST must not be replayed"
    );
    assert_eq!(
        server.count(202),
        3,
        "each distinct B event must POST exactly once"
    );
    assert_eq!(
        get_offset(&fixture.mirror_db, "thread-a")
            .unwrap()
            .unwrap()
            .cursor,
        0
    );
}

#[tokio::test]
async fn background_discovers_late_b_while_an_existing_http_a_is_stalled() {
    let fixture = Fixture::new(false);
    let server = Server::new().await;
    let running = fixture.run_http(&server);
    timeout(HEALTHY_B_BUDGET, server.a_started.notified())
        .await
        .unwrap();
    fixture.append_b("B arrived after A was already in flight");
    timeout(HEALTHY_B_BUDGET, fixture.b_committed())
        .await
        .expect("late B must not wait for the stalled initial batch");
    running.stop().await;
    assert_eq!(server.count(201), 1);
    assert_eq!(server.count(202), 1);
}

struct RepeatedFailure {
    a_attempts: AtomicUsize,
    a_failed: Notify,
}

impl SessionMirrorSender for RepeatedFailure {
    fn send<'a>(
        &'a self,
        channel: u64,
        _identity: &'a SessionMirrorDeliveryIdentity,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            if channel == 201 {
                self.a_attempts.fetch_add(1, Ordering::SeqCst);
                self.a_failed.notify_one();
                Err("persistent A fixture failure".into())
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test(start_paused = true)]
async fn background_a_retry_backoff_does_not_delay_new_b_events() {
    let fixture = Fixture::new(false);
    let sender = Arc::new(RepeatedFailure {
        a_attempts: AtomicUsize::new(0),
        a_failed: Notify::new(),
    });
    let running = fixture.run(sender.clone());
    timeout(Duration::from_secs(100), async {
        while sender.a_attempts.load(Ordering::SeqCst) < 6 {
            sender.a_failed.notified().await;
        }
    })
    .await
    .expect("fixture must reach A's 30s retry backoff");
    fixture.append_b("B arrived while A has a 30s retry delay");
    timeout(HEALTHY_B_BUDGET, fixture.b_committed())
        .await
        .expect("A's target-local retry backoff must not delay B by 30s");
    running.stop().await;
}
