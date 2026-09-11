use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::{Duration, timeout},
};
pub struct Reply {
    pub status: &'static str,
    pub body: String,
    pub header: String,
    pub disconnect: bool,
}
impl Reply {
    pub fn json(status: &'static str, body: &str) -> Self {
        Self {
            status,
            body: body.into(),
            header: String::new(),
            disconnect: false,
        }
    }
}
pub async fn server(replies: Vec<Reply>) -> (String, JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut records = Vec::new();
        for reply in replies {
            let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            records.push(
                timeout(Duration::from_secs(5), read_request(&mut stream))
                    .await
                    .unwrap(),
            );
            if !reply.disconnect {
                let response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                    reply.status,
                    reply.body.len(),
                    reply.header,
                    reply.body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        }
        assert!(
            timeout(Duration::from_millis(250), listener.accept())
                .await
                .is_err(),
            "unexpected duplicate request or redirect"
        );
        records
    });
    (format!("http://{address}/api/v10"), task)
}
async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 8192];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "request unexpectedly closed");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < 1024 * 1024);
        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            assert!(
                !headers.contains("transfer-encoding: chunked"),
                "test requires a bounded content length"
            );
            let length = headers
                .lines()
                .find_map(|v| v.strip_prefix("content-length:"))
                .map_or(0, |v| v.trim().parse::<usize>().unwrap());
            if bytes.len() >= end + 4 + length {
                return bytes;
            }
        }
    }
}
