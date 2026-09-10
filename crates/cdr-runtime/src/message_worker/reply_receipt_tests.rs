use super::*;
use crate::test_support::{app_fixture, message_fixture::MessageFixture};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

struct ReplyServer {
    address: String,
    count: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

async fn read_post(socket: &mut TcpStream) {
    let mut raw = Vec::new();
    loop {
        let mut bytes = [0u8; 4096];
        let read = socket.read(&mut bytes).await.unwrap();
        assert_ne!(read, 0);
        raw.extend_from_slice(&bytes[..read]);
        assert!(raw.len() < 32_768);
        if let Some(end) = raw.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&raw[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if raw.len() >= end + 4 + length {
                break;
            }
        }
    }
    assert!(raw.starts_with(b"POST /api/v10/channels/42/messages "));
}

async fn start_http(malformed: bool) -> ReplyServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            read_post(&mut socket).await;
            observed.fetch_add(1, Ordering::SeqCst);
            if malformed {
                socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 8\r\nconnection: close\r\n\r\nnot-json").await.unwrap();
            } // Otherwise the accepted POST loses its response.
        }
    });
    ReplyServer {
        address,
        count,
        stop,
        task,
    }
}

async fn verify_uncertain_reply(malformed: bool) {
    let temp = tempfile::tempdir().unwrap();
    let server = start_http(malformed).await;
    let http = Arc::new(
        Client::builder()
            .token("fixture-token".into())
            .proxy(server.address.clone(), true)
            .ratelimiter(None)
            .timeout(Duration::from_secs(1))
            .build(),
    );
    let fixture = MessageFixture::new(&temp, http).await;
    let db = fixture.executor.mirror_db();
    let context = fixture.context(temp.path());
    let result = process_admitted_gateway_message(fixture.admit("!help"), &context).await;
    let saved = cdr_store::ingress::get(db, "message:801").unwrap().unwrap();
    let reopened_api = DiscordHttp::new(
        Arc::new(
            Client::builder()
                .token("fixture-token".into())
                .proxy(server.address, true)
                .ratelimiter(None)
                .timeout(Duration::from_secs(1))
                .build(),
        ),
        Id::new(1),
    );
    let replay = reply_delivery::deliver_reply_text(
        db,
        &reopened_api,
        Id::new(42),
        Id::new(801),
        MessageReplyKind::ActionResult,
        saved.outcome.as_ref().unwrap()["response"]
            .as_str()
            .unwrap(),
    )
    .await;
    fixture.server.close().await.unwrap();
    server.stop.send(()).unwrap();
    server.task.await.unwrap();
    assert!(
        result.is_err(),
        "undecodable HTTP 200 must not confirm ordinary reply custody"
    );
    assert!(!saved.confirmation_delivered);
    assert!(
        replay.is_err(),
        "new sender must retain the uncertain receipt"
    );
    assert_eq!(cdr_store::delivery_receipt::unknown_count(db).unwrap(), 1);
    assert_eq!(
        server.count.load(Ordering::SeqCst),
        1,
        "ambiguous POST must not be retried"
    );
    assert!(
        !app_fixture::rpc_log(&temp.path().join("rpc.jsonl"))
            .iter()
            .any(|rpc| rpc["method"] == "turn/start")
    );
}

#[tokio::test]
async fn ordinary_reply_requires_readable_message_receipt() {
    verify_uncertain_reply(true).await;
}

#[tokio::test]
async fn ordinary_reply_does_not_retry_lost_http_response() {
    verify_uncertain_reply(false).await;
}

#[tokio::test]
async fn component_reply_receipt_survives_restart_and_covers_the_button_payload() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let gate = crate::test_support::http_gate::start().await;
    let http = Arc::new(
        Client::builder()
            .token("fixture-token".into())
            .proxy(gate.address, true)
            .ratelimiter(None)
            .build(),
    );
    let original = DiscordHttp::new(http.clone(), Id::new(1));
    let components =
        vec![cdr_discord::components::busy_button_row("0123456789abcdef01234567", true).unwrap()];
    gate.release.send(()).unwrap();
    reply_delivery::send_reply_once(
        &db,
        &original,
        Id::new(42),
        Id::new(801),
        MessageReplyKind::ActionResult,
        "Choose",
        &components,
    )
    .await
    .unwrap();
    gate.entered.await.unwrap();
    drop(original);
    let restarted = DiscordHttp::new(http, Id::new(1));
    reply_delivery::send_reply_once(
        &db,
        &restarted,
        Id::new(42),
        Id::new(801),
        MessageReplyKind::ActionResult,
        "Choose",
        &components,
    )
    .await
    .unwrap();
    let changed =
        vec![cdr_discord::components::busy_button_row("0123456789abcdef01234567", false).unwrap()];
    let conflict = reply_delivery::send_reply_once(
        &db,
        &restarted,
        Id::new(42),
        Id::new(801),
        MessageReplyKind::ActionResult,
        "Choose",
        &changed,
    )
    .await
    .unwrap_err();
    assert!(conflict.to_string().contains("content changed"));
    reply_delivery::send_reply_once(
        &db,
        &restarted,
        Id::new(42),
        Id::new(802),
        MessageReplyKind::ActionResult,
        "Choose",
        &components,
    )
    .await
    .unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0]["components"],
        serde_json::to_value(&components).unwrap()
    );
    assert_ne!(posts[0]["nonce"], posts[1]["nonce"]);
    assert_eq!(cdr_store::delivery_receipt::unknown_count(&db).unwrap(), 0);
}
