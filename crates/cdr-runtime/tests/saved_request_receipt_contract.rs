use cdr_discord::{
    http::DiscordHttp,
    text::{split_delivery_chunks, split_exact_delivery_chunks},
};
use cdr_runtime::message_worker::reply_delivery::{
    MessageReplyIdentity, MessageReplyKind, deliver_reply_text,
};
use cdr_store::delivery_receipt::{self, ReceiptState};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use twilight_model::id::Id;
#[path = "support/approval_http.rs"]
mod http;

#[tokio::test]
async fn saved_request_keeps_legacy_identity_and_never_resends_unknown_or_changed_chunks() {
    for mode in ["legacy_conflict", "unknown", "delivered"] {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("mirror.sqlite");
        let source = Id::new(801);
        let identity = MessageReplyIdentity::new(source, MessageReplyKind::SavedRequest);
        assert_eq!(
            identity,
            MessageReplyIdentity::new(source, MessageReplyKind::ActionResult)
        );
        let text = "한  ".repeat(2500);
        let current = split_exact_delivery_chunks(&text, true);
        let legacy = split_delivery_chunks(&text, true);
        assert_ne!(
            current[0], legacy[0],
            "fixture must expose the prior boundary trim"
        );
        let seeded = if mode == "delivered" {
            current.len()
        } else {
            1
        };
        for (index, chunk) in current.iter().enumerate().take(seeded) {
            let content = if mode == "legacy_conflict" {
                &legacy[index]
            } else {
                chunk
            };
            let key =
                serde_json::to_string(&(42_u64, identity.domain(), identity.logical_key(), index))
                    .unwrap();
            let hash = hex::encode(Sha256::digest(content.as_bytes()));
            assert_eq!(
                delivery_receipt::begin(&db, &key, &hash).unwrap(),
                ReceiptState::New
            );
            if mode != "unknown" {
                assert!(delivery_receipt::confirm(&db, &key, &(index + 1).to_string()).unwrap());
            }
        }
        let fixture = http::start().await;
        let api = DiscordHttp::new(
            Arc::new(
                twilight_http::Client::builder()
                    .proxy(fixture.address, true)
                    .ratelimiter(None)
                    .build(),
            ),
            Id::new(1),
        );
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            deliver_reply_text(
                &db,
                &api,
                Id::new(42),
                source,
                MessageReplyKind::SavedRequest,
                &text,
            ),
        )
        .await
        .unwrap();
        if mode == "delivered" {
            assert_eq!(result.unwrap(), current.len());
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.part, 1);
            assert_eq!(error.attempts, 1);
            assert!(error.source.to_string().contains(if mode == "unknown" {
                "send outcome unknown"
            } else {
                "delivery content changed"
            }));
        }
        fixture.stop.send(()).unwrap();
        assert!(
            fixture.task.await.unwrap().is_empty(),
            "no new HTTP send for {mode}"
        );
        let count: i64 = rusqlite::Connection::open(&db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM codex_delivery_receipts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count,
            i64::try_from(seeded).unwrap(),
            "no later chunk was admitted for {mode}"
        );
    }
}
