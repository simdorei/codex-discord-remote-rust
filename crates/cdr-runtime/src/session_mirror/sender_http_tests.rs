use super::*;
use crate::test_support::http_gate;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn confirmed_mirror_receipt_survives_sender_restart_and_new_turn_still_sends() {
    let gate = http_gate::start().await;
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let http = Arc::new(
        Client::builder()
            .token("test-token".into())
            .proxy(gate.address, true)
            .ratelimiter(None)
            .timeout(Duration::from_secs(2))
            .build(),
    );
    let sender = DiscordSessionMirrorSender::new(http.clone(), db.clone());
    gate.release.send(()).unwrap();
    let first = SessionMirrorDeliveryIdentity::event("thread", "turn-one");
    sender.send(42, &first, "Final\nsame reply").await.unwrap();
    gate.entered.await.unwrap();
    let restarted = DiscordSessionMirrorSender::new(http, db.clone());
    restarted
        .send(42, &first, "Final\nsame reply")
        .await
        .unwrap();
    restarted
        .send(
            42,
            &SessionMirrorDeliveryIdentity::event("thread", "turn-two"),
            "Final\nsame reply",
        )
        .await
        .unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(
        posts.len(),
        2,
        "restart replay must not create a third message"
    );
    assert_eq!(posts[0]["content"], "Final\nsame reply");
    assert_eq!(posts[1]["content"], "Final\nsame reply");
    assert_ne!(posts[0]["nonce"], posts[1]["nonce"]);
    assert_eq!(cdr_store::delivery_receipt::unknown_count(&db).unwrap(), 0);
}

#[tokio::test]
async fn accepted_but_unreadable_receipt_is_not_mirror_success() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::builder()
        .token("test-token".into())
        .proxy(listener.local_addr().unwrap().to_string(), true)
        .ratelimiter(None)
        .timeout(Duration::from_secs(1))
        .build();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 8192];
        assert!(socket.read(&mut buffer).await.unwrap() > 0);
        socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 8\r\nconnection: close\r\n\r\nnot-json").await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let sender = DiscordSessionMirrorSender::new(Arc::new(client), db.clone());
    let result = sender
        .send(
            42,
            &SessionMirrorDeliveryIdentity::event("thread", "event"),
            "Final\nreply",
        )
        .await;
    server.await.unwrap();
    assert!(
        result.is_err(),
        "an unreadable receipt must not advance the mirror cursor as delivered"
    );
    assert_eq!(cdr_store::delivery_receipt::unknown_count(&db).unwrap(), 1);
    // A reconstructed sender uses the same durable fence, not an in-memory cache.
    let restarted = DiscordSessionMirrorSender::new(sender.http.clone(), db);
    let error = restarted
        .send(
            42,
            &SessionMirrorDeliveryIdentity::event("thread", "event"),
            "Final\nreply",
        )
        .await
        .unwrap_err();
    assert!(error.contains("outcome unknown"), "{error}");
}
