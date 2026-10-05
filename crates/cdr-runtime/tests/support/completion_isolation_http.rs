//! Concurrent loopback HTTP oracle. Only A's first message response is delayed.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};

#[derive(Clone, Debug)]
pub struct Post {
    pub channel: u64,
    pub content: String,
}

pub struct Server {
    pub address: String,
    pub posts: Arc<Mutex<Vec<Post>>>,
    pub entered: oneshot::Receiver<()>,
    pub a_replied: Arc<AtomicBool>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Server {
    pub async fn close(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap();
    }
}

pub async fn start() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let posts = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&posts);
    let a_replied = Arc::new(AtomicBool::new(false));
    let replied = Arc::clone(&a_replied);
    let (entered, received) = oneshot::channel();
    let entered = Arc::new(Mutex::new(Some(entered)));
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                result = tasks.join_next(), if !tasks.is_empty() => { result.unwrap().unwrap(); },
                incoming = listener.accept(), if tasks.len() < 8 => {
                    let (socket, _) = incoming.unwrap();
                    tasks.spawn(serve(socket, Arc::clone(&recorded),
                        Arc::clone(&entered), Arc::clone(&replied)));
                }
            }
        }
        tasks.abort_all();
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                assert!(error.is_cancelled(), "{error}");
            }
        }
    });
    Server {
        address,
        posts,
        entered: received,
        a_replied,
        stop,
        task,
    }
}

async fn read_request(socket: &mut TcpStream) -> (String, Vec<u8>) {
    let mut raw = Vec::new();
    loop {
        let mut buffer = [0_u8; 1024];
        let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(count, 0, "request ended before headers/body");
        raw.extend_from_slice(&buffer[..count]);
        assert!(raw.len() < 16_384, "fixture request exceeds its bound");
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8(raw[..end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if raw.len() >= end + 4 + length {
                return (headers, raw[end + 4..end + 4 + length].to_vec());
            }
        }
    }
}

async fn serve(
    mut socket: TcpStream,
    posts: Arc<Mutex<Vec<Post>>>,
    entered: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    a_replied: Arc<AtomicBool>,
) {
    let (headers, body) = read_request(&mut socket).await;
    let route = headers.lines().next().unwrap();
    assert!(route.starts_with("POST /api/v10/channels/"), "{route}");
    if route.contains("/typing ") {
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
        return;
    }
    assert!(route.contains("/messages "), "{route}");
    let channel = route
        .split("/channels/")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(
        [42, 43].contains(&channel),
        "unexpected fixture destination"
    );
    let body: Value = serde_json::from_slice(&body).unwrap();
    posts.lock().unwrap().push(Post {
        channel,
        content: body["content"].as_str().unwrap().into(),
    });
    let first_a = if channel == 42 {
        entered.lock().unwrap().take()
    } else {
        None
    };
    if let Some(entered) = first_a {
        entered.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        a_replied.store(true, Ordering::SeqCst);
    }
    let body = json!({"id":(channel+1000).to_string(),"channel_id":channel.to_string(),
        "author":{"id":"1","username":"fixture","discriminator":"0001","avatar":null,"bot":false},
        "content":"accepted","timestamp":"2020-02-02T02:02:02.020000+00:00","edited_timestamp":null,
        "tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],
        "embeds":[],"pinned":false,"type":0})
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}
