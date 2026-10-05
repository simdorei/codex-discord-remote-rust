//! A concurrent loopback peer, with explicit stalls and accepted-but-lost responses.
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, oneshot},
    task::{JoinHandle, JoinSet},
};

#[derive(Clone, Debug)]
pub struct Post {
    pub channel: u64,
    pub content: String,
}

struct State {
    posts: Arc<Mutex<Vec<Post>>>,
    pause: HashSet<u64>,
    drop_once: Mutex<HashSet<u64>>,
    released: AtomicBool,
    wake: Notify,
}

pub struct Server {
    pub address: String,
    pub posts: Arc<Mutex<Vec<Post>>>,
    state: Arc<State>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}
impl Server {
    pub fn release(&self) {
        self.state.released.store(true, Ordering::SeqCst);
        self.state.wake.notify_waiters();
    }
    pub async fn close(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap();
    }
}

pub async fn start(
    pause: impl IntoIterator<Item = u64>,
    drop_once: impl IntoIterator<Item = u64>,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let posts = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(State {
        posts: Arc::clone(&posts),
        pause: pause.into_iter().collect(),
        drop_once: Mutex::new(drop_once.into_iter().collect()),
        released: AtomicBool::new(false),
        wake: Notify::new(),
    });
    let serving = Arc::clone(&state);
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut calls = JoinSet::new();
        loop {
            tokio::select! {
                _=&mut stopped=>break,
                result=calls.join_next(),if !calls.is_empty()=>{result.unwrap().unwrap();},
                incoming=listener.accept(),if calls.len()<64=>{
                    let (socket,_)=incoming.unwrap();
                    calls.spawn(serve(socket,Arc::clone(&serving)));
                }
            }
        }
        calls.abort_all();
        while let Some(result) = calls.join_next().await {
            if let Err(error) = result {
                assert!(error.is_cancelled(), "{error}");
            }
        }
    });
    Server {
        address,
        posts,
        state,
        stop,
        task,
    }
}

async fn request(socket: &mut TcpStream) -> Option<(String, Value)> {
    let mut raw = Vec::new();
    loop {
        let mut buffer = [0; 1024];
        let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
            .await
            .ok()?
            .ok()?;
        if count == 0 {
            return None;
        }
        raw.extend_from_slice(&buffer[..count]);
        assert!(raw.len() < 16_384);
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
                let body = if length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&raw[end + 4..end + 4 + length]).unwrap()
                };
                return Some((headers, body));
            }
        }
    }
}

async fn serve(mut socket: TcpStream, state: Arc<State>) {
    let Some((headers, body)) = request(&mut socket).await else {
        return;
    };
    let route = headers.lines().next().unwrap();
    assert!(route.starts_with("POST /api/v10/channels/"), "{route}");
    if route.contains("/typing ") {
        let _ = socket
            .write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await;
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
    let message = {
        let mut posts = state.posts.lock().unwrap();
        assert!(posts.len() < 512, "fixture POST cap");
        posts.push(Post {
            channel,
            content: body["content"].as_str().unwrap().into(),
        });
        posts.len() + 1000
    };
    if state.drop_once.lock().unwrap().remove(&channel) {
        return;
    }
    if state.pause.contains(&channel) {
        loop {
            let wake = state.wake.notified();
            if state.released.load(Ordering::SeqCst) {
                break;
            }
            wake.await;
        }
    }
    let body = json!({"id":message.to_string(),"channel_id":channel.to_string(),
        "author":{"id":"1","username":"fixture","discriminator":"0001","avatar":null,"bot":false},
        "content":"accepted","timestamp":"2020-02-02T02:02:02.020000+00:00","edited_timestamp":null,
        "tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],
        "embeds":[],"pinned":false,"type":0})
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
}
