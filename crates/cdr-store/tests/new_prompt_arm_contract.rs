use cdr_store::ingress::{self, IngressKind, NewIngress};
use serde_json::json;
use std::path::Path;

fn request(id: i64, channel: i64, user: i64, content: &str) -> NewIngress {
    let plan = if content == "!new" {
        json!({"Execute":{"New":{"prompt":""}}})
    } else if content.starts_with('!') {
        json!({"Execute":"Help"})
    } else {
        json!({"Execute":{"Ask":{"prompt":content}}})
    };
    NewIngress {
        ingress_id: format!("message:{id}"),
        kind: IngressKind::Message,
        event_id: Some(id),
        application_id: None,
        channel_id: channel,
        owner_user_id: user,
        source_message_id: Some(id),
        payload: json!({"version":1,"content":content,
            "plan":plan,"author_is_bot":false,"routing":{"mirrored_target":"original"}}),
        target_thread_id: Some("original".into()),
        canonical_owner: None,
        now: 100.0,
    }
}
fn admit(db: &Path, id: i64, channel: i64, user: i64, content: &str) -> serde_json::Value {
    ingress::admit(db, &request(id, channel, user, content))
        .unwrap()
        .record
        .unwrap()
        .payload
}

#[test]
fn bare_new_and_next_prompt_are_persisted_as_distinct_operations() {
    let t = tempfile::tempdir().unwrap();
    let db = t.path().join("mirror.sqlite");
    let arm = admit(&db, 30, 99, 20, "!new");
    assert_eq!(
        arm["plan"]["Respond"],
        "새 대화를 준비했습니다. 같은 방에 첫 요청을 보내주세요."
    );
    let next = admit(&db, 31, 99, 20, "첫 요청");
    assert_eq!(next["plan"]["Execute"]["New"]["prompt"], "첫 요청");
    assert_eq!(next["new_prompt_arm_ref"], "message:30");
    assert_eq!(
        ingress::new_command_prompt(&ingress::get(&db, "message:31").unwrap().unwrap()),
        Some("첫 요청")
    );
    let duplicate = ingress::admit(&db, &request(31, 99, 20, "edited replay")).unwrap();
    assert!(!duplicate.created);
    assert_eq!(duplicate.record.unwrap().payload, next);
    assert!(
        admit(&db, 32, 99, 20, "후속")
            .pointer("/plan/Execute/Ask")
            .is_some()
    );
}

#[test]
fn pending_new_is_scoped_and_does_not_consume_commands_or_old_history() {
    let t = tempfile::tempdir().unwrap();
    let db = t.path().join("mirror.sqlite");
    admit(&db, 30, 99, 20, "!new");
    for (id, ch, user, text) in [
        (29, 99, 20, "old"),
        (31, 100, 20, "other room"),
        (32, 99, 21, "other user"),
        (33, 99, 20, "!help"),
    ] {
        assert!(
            admit(&db, id, ch, user, text)
                .get("new_prompt_arm_ref")
                .is_none()
        );
    }
    assert_eq!(
        admit(&db, 34, 99, 20, "first")["new_prompt_arm_ref"],
        "message:30"
    );
}

#[test]
fn failed_ingress_insert_rolls_back_consumption() {
    let t = tempfile::tempdir().unwrap();
    let db = t.path().join("mirror.sqlite");
    admit(&db, 30, 99, 20, "!new");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch("CREATE TRIGGER reject_new BEFORE INSERT ON discord_ingress_journal WHEN NEW.event_id=31 BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(ingress::admit(&db, &request(31, 99, 20, "first")).is_err());
    c.execute_batch("DROP TRIGGER reject_new;").unwrap();
    assert_eq!(
        admit(&db, 31, 99, 20, "first")["new_prompt_arm_ref"],
        "message:30"
    );
}

#[test]
fn concurrent_messages_consume_one_arm_once() {
    let t = tempfile::tempdir().unwrap();
    let db = t.path().join("mirror.sqlite");
    admit(&db, 30, 99, 20, "!new");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles = (31..39)
        .map(|id| {
            let path = db.clone();
            let gate = barrier.clone();
            std::thread::spawn(move || {
                gate.wait();
                admit(&path, id, 99, 20, "first")
            })
        })
        .collect::<Vec<_>>();
    let count = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|p| p.get("new_prompt_arm_ref").is_some())
        .count();
    assert_eq!(count, 1);
}
