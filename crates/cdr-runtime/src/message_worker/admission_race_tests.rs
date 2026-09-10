use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use cdr_discord::interaction_access::InteractionAccessPolicy;
use twilight_model::channel::Message;

use super::admit_message_candidate_at;
use crate::config::{CliOptions, RuntimeConfig};
use crate::message_worker::classification::{MessageClassification, classify_gateway_message};

fn message() -> Message {
    serde_json::from_value(serde_json::json!({
        "attachments": [],
        "author": {
            "avatar": null,
            "bot": false,
            "discriminator": "0001",
            "id": "3",
            "username": "tester"
        },
        "channel_id": "2",
        "content": "race",
        "edited_timestamp": null,
        "embeds": [],
        "id": "207",
        "mention_everyone": false,
        "mention_roles": [],
        "mentions": [],
        "pinned": false,
        "timestamp": "2020-02-02T02:02:02.020000+00:00",
        "tts": false,
        "type": 0
    }))
    .expect("valid gateway message fixture")
}

#[test]
fn mca_05_sixteen_candidates_have_one_row_winner_and_processor() {
    const WORKERS: usize = 16;
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    let config = Arc::new(
        RuntimeConfig::from_map(
            &BTreeMap::from([
                ("DISCORD_BOT_TOKEN".into(), "test-token".into()),
                ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
            ]),
            CliOptions::default(),
        )
        .expect("valid test config"),
    );
    let policy = Arc::new(InteractionAccessPolicy {
        allow_all_channels: true,
        ..InteractionAccessPolicy::default()
    });
    let barrier = Arc::new(Barrier::new(WORKERS));
    let processors = Arc::new(AtomicUsize::new(0));
    let handles = (0..WORKERS)
        .map(|_| {
            let database = database.clone();
            let config = Arc::clone(&config);
            let policy = Arc::clone(&policy);
            let barrier = Arc::clone(&barrier);
            let processors = Arc::clone(&processors);
            thread::spawn(move || {
                let classification =
                    classify_gateway_message(message(), &database, &config, &policy, None)
                        .expect("classification succeeds");
                let MessageClassification::Candidate(candidate) = classification else {
                    panic!("race input must be a candidate");
                };
                barrier.wait();
                if let Some(admitted) =
                    admit_message_candidate_at(candidate, UNIX_EPOCH + Duration::from_secs(100))
                        .expect("atomic admission succeeds")
                {
                    let _ = admitted
                        .into_processing_parts(&database)
                        .expect("winner token matches database");
                    processors.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().expect("candidate worker did not panic");
    }

    let connection = rusqlite::Connection::open(&database).expect("open message database");
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM discord_processed_messages WHERE message_id = 207",
            [],
            |row| row.get(0),
        )
        .expect("count durable race winners");
    assert_eq!(rows, 1);
    assert_eq!(processors.load(Ordering::SeqCst), 1);
}
