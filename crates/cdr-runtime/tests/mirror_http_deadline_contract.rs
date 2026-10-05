use cdr_runtime::mirror_sync::{
    DiscordMirrorTransport, MirrorChannel, MirrorFuture, MirrorInventoryThread, MirrorSyncError,
    MirrorSynchronizer, MirrorTransport,
};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::{JoinHandle, JoinSet},
};
use twilight_http::Client;
use twilight_model::{channel::ChannelType, id::Id};

#[derive(Clone, Copy)]
enum Mode {
    BodyStall,
    RateLimit,
    Success,
    Missing,
    Forbidden,
}

struct Server {
    address: SocketAddr,
    mirror_requests: Arc<AtomicUsize>,
    other_requests: Arc<AtomicUsize>,
    started: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Server {
    async fn new(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mirror_requests = Arc::new(AtomicUsize::new(0));
        let other_requests = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let mirror = mirror_requests.clone();
        let other = other_requests.clone();
        let signal = started.clone();
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        tasks.spawn(serve(socket, mode, mirror.clone(), other.clone(), signal.clone()));
                    }
                    Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
                }
            }
        });
        Self {
            address,
            mirror_requests,
            other_requests,
            started,
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
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn request_headers(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let end = loop {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0, "fixture request closed before headers");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 16_384);
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(length < 16_384);
    while bytes.len() < end + length {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0, "fixture request closed before body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    headers
}

async fn serve(
    mut socket: TcpStream,
    mode: Mode,
    mirror: Arc<AtomicUsize>,
    other: Arc<AtomicUsize>,
    started: Arc<Notify>,
) {
    let request = request_headers(&mut socket).await;
    if request.contains("/channels/101/messages ") {
        other.fetch_add(1, Ordering::SeqCst);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await
            .unwrap();
        return;
    }
    mirror.fetch_add(1, Ordering::SeqCst);
    match mode {
        Mode::BodyStall => {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n{").await.unwrap();
            started.notify_one();
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        Mode::RateLimit => {
            let body =
                r#"{"message":"fixture shared rate limit","retry_after":30.0,"global":false}"#;
            let response = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-RateLimit-Scope: shared\r\nX-RateLimit-Bucket: mirror-fixture\r\nRetry-After: 30\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            started.notify_one();
        }
        Mode::Success => {
            let body = if request.contains("/guilds/1/channels ") {
                r#"[{"id":"20","type":0,"guild_id":"1","parent_id":"10","name":"alpha"}]"#
            } else if request.starts_with("POST ") {
                r#"{"id":"1001","type":11,"guild_id":"1","parent_id":"20","name":"created"}"#
            } else {
                r#"{"id":"30","type":11,"guild_id":"1","parent_id":"20","name":"existing"}"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        Mode::Missing | Mode::Forbidden => {
            let (status, body) = if matches!(mode, Mode::Missing) {
                (
                    "404 Not Found",
                    r#"{"code":10003,"message":"Unknown Channel"}"#,
                )
            } else {
                (
                    "403 Forbidden",
                    r#"{"code":50013,"message":"Missing Permissions"}"#,
                )
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    }
}

async fn bounded_error<T: std::fmt::Debug>(
    mut task: JoinHandle<Result<T, MirrorSyncError>>,
) -> MirrorSyncError {
    let result = if let Ok(joined) = tokio::time::timeout(Duration::from_secs(12), &mut task).await
    {
        joined.unwrap()
    } else {
        task.abort();
        let _ = task.await;
        panic!("mirror HTTP operation exceeded the 10s total deadline and the 12s test watchdog");
    };
    result.unwrap_err()
}

async fn read_deadline(mode: Mode) {
    let server = Server::new(mode).await;
    let client = server.client();
    let transport = DiscordMirrorTransport::new(client.clone());
    let task = tokio::spawn(async move { transport.channel(30).await });
    tokio::time::timeout(Duration::from_secs(5), server.started.notified())
        .await
        .unwrap();
    let other_start = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(5),
        client.create_message(Id::new(101)).content("independent B"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(other_start.elapsed() < Duration::from_secs(5));
    assert_eq!(server.other_requests.load(Ordering::SeqCst), 1);
    let error = bounded_error(task).await;
    assert!(error.to_string().contains("deadline"), "{error}");
    assert_eq!(server.mirror_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mirror_http_body_deadline() {
    read_deadline(Mode::BodyStall).await;
}

#[tokio::test]
async fn mirror_http_rate_limit_deadline() {
    read_deadline(Mode::RateLimit).await;
}

#[tokio::test]
async fn mirror_http_successful_operations_keep_existing_results() {
    let server = Server::new(Mode::Success).await;
    let transport = DiscordMirrorTransport::new(server.client());
    let channel = transport.channel(30).await.unwrap().unwrap();
    assert_eq!(channel.id, 30);
    assert_eq!(transport.channels(1).await.unwrap()[0].id, 20);
    assert_eq!(
        transport
            .create(1, Some(20), ChannelType::PublicThread, "created", None)
            .await
            .unwrap()
            .id,
        1001
    );
    transport
        .update(&channel, "renamed", None, false)
        .await
        .unwrap();
    transport.delete(30).await.unwrap();
    assert_eq!(server.mirror_requests.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn mirror_http_missing_and_forbidden_remain_distinct() {
    let missing = Server::new(Mode::Missing).await;
    let transport = DiscordMirrorTransport::new(missing.client());
    assert!(transport.channel(30).await.unwrap().is_none());
    transport.delete(30).await.unwrap();
    let denied = Server::new(Mode::Forbidden).await;
    let transport = DiscordMirrorTransport::new(denied.client());
    assert!(transport.channel(30).await.is_err());
    assert!(matches!(
        transport.delete(30).await,
        Err(MirrorSyncError::DeleteRejected(_))
    ));
    assert_eq!(missing.mirror_requests.load(Ordering::SeqCst), 2);
    assert_eq!(denied.mirror_requests.load(Ordering::SeqCst), 2);
}

struct NewRemote(DiscordMirrorTransport);
impl MirrorTransport for NewRemote {
    fn channel(&self, id: u64) -> MirrorFuture<'_, Option<MirrorChannel>> {
        Box::pin(async move {
            Ok((id == 20).then(|| MirrorChannel {
                id,
                guild_id: Some(1),
                parent_id: Some(10),
                kind: ChannelType::GuildText,
                name: "alpha".to_owned(),
                topic: None,
                archived: false,
            }))
        })
    }
    fn channels(&self, _: u64) -> MirrorFuture<'_, Vec<MirrorChannel>> {
        panic!("no global sync")
    }
    fn thread_inventory(&self, _: u64, _: u64) -> MirrorFuture<'_, Vec<MirrorInventoryThread>> {
        panic!("no inventory")
    }
    fn delete(&self, _: u64) -> MirrorFuture<'_, ()> {
        panic!("no rollback delete")
    }
    fn update<'a>(
        &'a self,
        _: &'a MirrorChannel,
        _: &'a str,
        _: Option<&'a str>,
        _: bool,
    ) -> MirrorFuture<'a, ()> {
        panic!("no update")
    }
    fn create<'a>(
        &'a self,
        guild: u64,
        parent: Option<u64>,
        kind: ChannelType,
        name: &'a str,
        topic: Option<&'a str>,
    ) -> MirrorFuture<'a, MirrorChannel> {
        self.0.create(guild, parent, kind, name, topic)
    }
}

#[tokio::test]
async fn mirror_http_create_deadline_preserves_unknown_custody() {
    let server = Server::new(Mode::BodyStall).await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_project(&path, "C:/repos/alpha", "alpha", 20, 1.0, |a, b| a == b)
        .unwrap();
    let remote = Arc::new(NewRemote(DiscordMirrorTransport::new(server.client())));
    let sync = MirrorSynchronizer::new(
        temp.path().join("state.sqlite"),
        path.clone(),
        remote.clone(),
        Some(1),
    );
    let task = tokio::spawn(async move {
        sync.link_new_thread(20, "thread-a", "new prompt", Some("C:/repos/alpha"))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), server.started.notified())
        .await
        .unwrap();
    let error = bounded_error(task).await;
    assert!(error.to_string().contains("deadline"), "{error}");
    let cold = MirrorSynchronizer::new(
        temp.path().join("state.sqlite"),
        path.clone(),
        remote,
        Some(1),
    );
    let retry = tokio::time::timeout(
        Duration::from_secs(5),
        cold.link_new_thread(20, "thread-a", "new prompt", Some("C:/repos/alpha")),
    )
    .await
    .unwrap();
    assert!(retry.is_err());
    assert_eq!(
        server.mirror_requests.load(Ordering::SeqCst),
        1,
        "unknown create must not POST again"
    );
    assert_eq!(
        cdr_store::mapping::thread_channels(&path, "thread-a").unwrap(),
        None
    );
    let phase: String = rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT phase FROM cdr_mirror_thread_creations WHERE thread_id='thread-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(phase, "attempted");
}
