//! Loopback Discord boundary; holds only the first response, records every POST.
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

pub(crate) struct HttpGate {
    pub address: String,
    pub entered: oneshot::Receiver<()>,
    pub release: oneshot::Sender<()>,
    pub stop: oneshot::Sender<()>,
    pub task: tokio::task::JoinHandle<Vec<Value>>,
}

pub(crate) async fn start() -> HttpGate {
    start_for_channels(vec![42]).await
}

pub(crate) async fn start_for_channels(channels: Vec<u64>) -> HttpGate {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (entered, receiver) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut entered = Some(entered);
        let mut released = Some(released);
        let mut posts: Vec<Value> = Vec::new();
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            let mut raw = Vec::new();
            let body_start = loop {
                let mut buffer = [0_u8; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                raw.extend_from_slice(&buffer[..count]);
                assert!(raw.len() < 16_384);
                if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&raw[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if raw.len() >= end + 4 + length {
                        break end + 4;
                    }
                }
            };
            let channel = channels
                .iter()
                .find(|id| {
                    raw.starts_with(format!("POST /api/v10/channels/{id}/messages ").as_bytes())
                })
                .copied()
                .expect("unexpected HTTP route");
            posts.push(serde_json::from_slice(&raw[body_start..]).unwrap());
            if channels.len() > 1 {
                posts.last_mut().unwrap()["test_channel"] = json!(channel);
            }
            if let Some(entered) = entered.take() {
                entered.send(()).unwrap();
                released.take().unwrap().await.unwrap();
            }
            let body = json!({"id":posts.len().to_string(),"channel_id":channel.to_string(),
                "author":{"id":"1","username":"test","discriminator":"0001","avatar":null,"bot":false},
                "content":"accepted","timestamp":"2020-02-02T02:02:02.020000+00:00","edited_timestamp":null,
                "tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],
                "embeds":[],"pinned":false,"type":0}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        posts
    });
    HttpGate {
        address,
        entered: receiver,
        release,
        stop,
        task,
    }
}
