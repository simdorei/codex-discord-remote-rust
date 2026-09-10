use cdr_app_server::ResidentAppServer;
use cdr_runtime::{action_executor::ActionExecutor, discord_dispatch::InboundInteractionWork};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub async fn release<B: cdr_runtime::queue_runner::TurnBackend>(
    work: InboundInteractionWork,
    executor: Arc<ActionExecutor<B>>,
    server: Arc<ResidentAppServer>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = Arc::new(
        twilight_http::Client::builder()
            .proxy(listener.local_addr().unwrap().to_string(), true)
            .ratelimiter(None)
            .build(),
    );
    let receipt = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut buffer = [0_u8; 2048];
            let size = socket.read(&mut buffer).await.unwrap();
            assert_ne!(size, 0);
            request.extend_from_slice(&buffer[..size]);
            assert!(request.len() < 16384);
            if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&request[..end]);
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        assert!(request.starts_with(b"PATCH /api/v10/webhooks/2/fixture/messages/@original "));
        socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}").await.unwrap();
    });
    let (send, recv) = tokio::sync::mpsc::channel(1);
    send.send(work).await.unwrap();
    drop(send);
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        cdr_runtime::interaction_worker::run_interaction_worker(recv, executor, server, http),
    )
    .await
    .unwrap();
    receipt.await.unwrap();
}
