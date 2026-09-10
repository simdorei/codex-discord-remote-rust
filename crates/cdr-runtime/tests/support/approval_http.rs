use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

pub struct HttpFixture {
    pub address: String,
    pub stop: oneshot::Sender<()>,
    pub task: tokio::task::JoinHandle<Vec<(bool, Value)>>,
}

pub async fn start() -> HttpFixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut traffic = Vec::new();
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
                assert!(raw.len() < 16384);
                if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&raw[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if raw.len() >= end + 4 + length {
                        break end + 4;
                    }
                }
            };
            let post = raw.starts_with(b"POST /api/v10/channels/42/messages ");
            assert!(
                post || raw.starts_with(b"PATCH /api/v10/webhooks/2/fixture/messages/@original ")
            );
            traffic.push((post, serde_json::from_slice(&raw[body_start..]).unwrap()));
            let body = json!({"id":traffic.len().to_string(),"channel_id":"42",
                "author":{"id":"1","username":"test","discriminator":"0001","avatar":null,"bot":false},
                "content":"accepted","timestamp":"2020-02-02T02:02:02.020000+00:00","edited_timestamp":null,
                "tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],
                "embeds":[],"pinned":false,"type":0}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        traffic
    });
    HttpFixture {
        address,
        stop,
        task,
    }
}
