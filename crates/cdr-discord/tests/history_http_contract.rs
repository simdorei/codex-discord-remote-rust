use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use cdr_discord::http::{DiscordHttp, DiscordHttpError};
use twilight_http::{Client, error::ErrorType};
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker},
};

fn discord_http(proxy: &str) -> DiscordHttp {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let client = Client::builder()
        .token("test-token".to_owned())
        .proxy(proxy.to_owned(), true)
        .ratelimiter(None)
        .timeout(Duration::from_secs(2))
        .build();

    DiscordHttp::new(Arc::new(client), Id::<ApplicationMarker>::new(1))
}

fn read_request_line(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("configure mock read timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 512];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).expect("read mock request");
        assert_ne!(count, 0, "request ended before headers completed");
        bytes.extend_from_slice(&chunk[..count]);
    }

    String::from_utf8(bytes)
        .expect("HTTP request headers are UTF-8")
        .lines()
        .next()
        .expect("request line exists")
        .to_owned()
}

fn mock_response(
    status: &str,
    body: &str,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback mock");
    let address = listener.local_addr().expect("read loopback address");
    let (request_tx, request_rx) = mpsc::channel();
    let status = status.to_owned();
    let body = body.to_owned();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept mock request");
        request_tx
            .send(read_request_line(&mut stream))
            .expect("record request line");
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write mock response");
    });

    (address.to_string(), request_rx, handle)
}

fn mock_disconnect() -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback mock");
    let address = listener.local_addr().expect("read loopback address");
    let (request_tx, request_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept mock request");
        request_tx
            .send(read_request_line(&mut stream))
            .expect("record request line");
    });

    (address.to_string(), request_rx, handle)
}

fn message_body() -> String {
    serde_json::json!([{
        "attachments": [],
        "author": {
            "avatar": null,
            "bot": false,
            "discriminator": "0001",
            "id": "3",
            "username": "tester"
        },
        "channel_id": "42",
        "content": "latest",
        "edited_timestamp": null,
        "embeds": [],
        "id": "99",
        "mention_everyone": false,
        "mention_roles": [],
        "mentions": [],
        "pinned": false,
        "timestamp": "2020-02-02T02:02:02.020000+00:00",
        "tts": false,
        "type": 0
    }])
    .to_string()
}

fn finish_mock(request_rx: &mpsc::Receiver<String>, handle: thread::JoinHandle<()>) {
    assert_eq!(
        request_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("mock observed request"),
        "GET /api/v10/channels/42/messages?limit=10 HTTP/1.1"
    );
    handle.join().expect("mock server completed");
}

#[tokio::test]
async fn public_history_fetch_requests_ten_and_decodes_twilight_messages() {
    let (proxy, request_rx, handle) = mock_response("200 OK", &message_body());

    let messages = discord_http(&proxy)
        .fetch_latest_channel_messages(Id::<ChannelMarker>::new(42))
        .await
        .expect("decode valid Discord history response");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id.get(), 99);
    assert_eq!(messages[0].channel_id.get(), 42);
    assert_eq!(messages[0].content, "latest");
    finish_mock(&request_rx, handle);
}

#[tokio::test]
async fn malformed_history_model_surfaces_without_a_fallback() {
    let (proxy, request_rx, handle) = mock_response("200 OK", "not-json");

    let error = discord_http(&proxy)
        .fetch_latest_channel_messages(Id::<ChannelMarker>::new(42))
        .await
        .expect_err("malformed Discord response must fail");

    assert!(matches!(error, DiscordHttpError::Model(_)));
    finish_mock(&request_rx, handle);
}

#[tokio::test]
async fn transport_failure_surfaces_without_a_fallback() {
    let (proxy, request_rx, handle) = mock_disconnect();

    let error = discord_http(&proxy)
        .fetch_latest_channel_messages(Id::<ChannelMarker>::new(42))
        .await
        .expect_err("closed transport must fail");

    assert!(matches!(
        error,
        DiscordHttpError::Request(source) if matches!(source.kind(), ErrorType::RequestError)
    ));
    finish_mock(&request_rx, handle);
}
