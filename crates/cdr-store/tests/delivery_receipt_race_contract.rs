use cdr_store::delivery_receipt::{ReceiptState, begin, release_rejected};
use std::sync::{Arc, Barrier};

#[test]
fn concurrent_producers_get_only_one_initial_or_retry_send_permit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    // Initialize the schema before racing independent database connections.
    assert_eq!(begin(&path, "seed", "hash").unwrap(), ReceiptState::New);
    for retry in [false, true] {
        if retry {
            assert!(release_rejected(&path, "shared").unwrap());
        }
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let barrier = Arc::clone(&barrier);
                    let path = &path;
                    scope.spawn(move || {
                        barrier.wait();
                        begin(path, "shared", "same payload").unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            results
                .iter()
                .filter(|value| **value == ReceiptState::New)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|value| **value == ReceiptState::Unknown)
                .count(),
            1
        );
    }
}

#[test]
fn legacy_receipt_schema_is_extended_without_losing_unknown_intent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.sqlite");
    begin(&path, "seed", "hash").unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("DROP TABLE codex_delivery_receipts;
        CREATE TABLE codex_delivery_receipts(receipt_key TEXT PRIMARY KEY,content_hash TEXT NOT NULL,message_id TEXT);
        INSERT INTO codex_delivery_receipts VALUES('old','hash',NULL);").unwrap();
    drop(connection);
    assert_eq!(begin(&path, "old", "hash").unwrap(), ReceiptState::Unknown);
}
