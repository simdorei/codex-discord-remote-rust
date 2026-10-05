use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

pub(super) struct HttpFixture {
    pub address: String,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Vec<(String, Value)>>,
}

impl HttpFixture {
    pub async fn finish(self) -> Vec<(String, Value)> {
        self.stop.send(()).unwrap();
        self.task.await.unwrap()
    }
}

pub(super) async fn start(unknown_first_post: bool) -> HttpFixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut traffic = Vec::new();
        let mut make_unknown = unknown_first_post;
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            let (request, body) = read_request(&mut socket).await;
            let post = request.starts_with("POST /api/v10/channels/42/messages ");
            assert!(post || request.starts_with("PATCH /api/v10/channels/42/messages/60 "));
            traffic.push((request, body));
            let body = if post && make_unknown {
                make_unknown = false;
                // The POST was accepted, but its response cannot prove a message ID.
                "{".to_owned()
            } else {
                json!({"id":"61","channel_id":"42",
                    "author":{"id":"1","username":"fixture","discriminator":"0001","avatar":null,"bot":true},
                    "content":"accepted","timestamp":"2020-02-02T02:02:02.020000+00:00",
                    "edited_timestamp":null,"tts":false,"mention_everyone":false,
                    "mentions":[],"mention_roles":[],"attachments":[],"embeds":[],"pinned":false,"type":0
                }).to_string()
            };
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await.unwrap();
        }
        traffic
    });
    HttpFixture {
        address,
        stop,
        task,
    }
}

async fn read_request(socket: &mut TcpStream) -> (String, Value) {
    let mut raw = Vec::new();
    loop {
        let mut buffer = [0_u8; 2048];
        let count =
            tokio::time::timeout(std::time::Duration::from_secs(2), socket.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_ne!(count, 0);
        raw.extend_from_slice(&buffer[..count]);
        assert!(raw.len() < 32_768);
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
                return (
                    header.lines().next().unwrap().to_owned(),
                    serde_json::from_slice(&raw[end + 4..end + 4 + length]).unwrap(),
                );
            }
        }
    }
}
