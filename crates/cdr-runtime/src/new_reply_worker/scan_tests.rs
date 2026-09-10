use super::*;
use cdr_store::{ingress::IngressKind, new_reply::Identity};
use serde_json::json;
use std::fmt::Write as _;

fn record() -> NewReply {
    NewReply {
        identity: Identity {
            ingress_id: "message:1".into(),
            job_id: "job".into(),
            thread_id: "thread".into(),
            cwd: "C:/project".into(),
            state_db: "unused".into(),
            channel_id: 100,
            origin_channel_id: 99,
            event_id: Some(1),
            kind: IngressKind::Message,
            creation_generation: 1,
            prompt_sha256: hex::encode(Sha256::digest("요청".as_bytes())),
            acknowledgement: "ack".into(),
        },
        turn_id: Some("first-turn".into()),
        accepted_at: Some(1.0),
        state: "pending".into(),
        version: 1,
        scan: json!({}),
        last_error: String::new(),
        confirmation_delivered: false,
        warning_due: 0,
        acknowledgement_recovery_allowed: false,
    }
}
fn meta() -> Value {
    json!({"type":"session_meta","payload":{"id":"thread","cwd":"C:/project"}})
}
fn turn(id: &str) -> Value {
    json!({"type":"turn_context","payload":{"turn_id":id}})
}
fn user(role: &str, text: &str) -> Value {
    json!({"type":"response_item","payload":{"role":role,"content":[{"text":text}]}})
}
fn write(path: &Path, events: &[Value]) {
    fs::write(
        path,
        events.iter().fold(String::new(), |mut text, event| {
            writeln!(text, "{event}").unwrap();
            text
        }),
    )
    .unwrap();
}

#[test]
fn exact_first_turn_user_input_is_required_not_later_identical_text() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    for (turn_id, role, text, expected) in [
        ("first-turn", "user", "요청", true),
        ("later-turn", "user", "요청", false),
        ("first-turn", "assistant", "요청", false),
        ("first-turn", "user", "다른 요청", false),
    ] {
        write(&path, &[meta(), turn(turn_id), user(role, text)]);
        assert_eq!(inspect_file(&path, &record()).unwrap().verified, expected);
    }
    write(&path, &[meta(), user("user", "요청")]);
    assert!(!inspect_file(&path, &record()).unwrap().verified);
    write(&path, &[turn("first-turn"), user("user", "요청")]);
    assert!(!inspect_file(&path, &record()).unwrap().verified);
}

#[test]
fn thread_and_cwd_mismatches_are_explicit_errors() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    for field in ["id", "cwd"] {
        let mut header = meta();
        header["payload"][field] = json!("wrong");
        write(&path, &[header, turn("first-turn"), user("user", "요청")]);
        assert!(
            inspect_file(&path, &record())
                .unwrap_err()
                .contains("identity mismatch")
        );
    }
}

#[test]
fn cursor_progresses_past_256_events_and_survives_serialization() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    let mut events = vec![meta(), turn("first-turn")];
    events.extend((0..700).map(|_| json!({"type":"ignored"})));
    events.push(user("user", "요청"));
    write(&path, &events);
    let mut saved = record();
    for attempt in 0..8 {
        let scan = inspect_file(&path, &saved).unwrap();
        if scan.verified {
            assert!(attempt >= 2);
            return;
        }
        assert!(
            scan.checkpoint["offset"].as_u64().unwrap()
                > saved.scan["offset"].as_u64().unwrap_or(0)
        );
        saved.scan = serde_json::from_str(&scan.checkpoint.to_string()).unwrap();
    }
    panic!("bounded batches never reached first input");
}

#[test]
fn partial_line_does_not_advance_and_completed_append_restarts_snapshot_safely() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    let prefix = format!("{}\n{}\n", meta(), turn("first-turn"));
    fs::write(&path, format!("{prefix}{{\"type\":")).unwrap();
    let mut saved = record();
    let scan = inspect_file(&path, &saved).unwrap();
    assert!(!scan.verified);
    assert_eq!(scan.checkpoint["offset"], prefix.len());
    saved.scan = scan.checkpoint;
    fs::write(&path, format!("{prefix}{}\n", user("user", "요청"))).unwrap();
    assert!(inspect_file(&path, &saved).unwrap().verified);
}

#[test]
fn replaced_or_truncated_file_cannot_reuse_prior_turn_context() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    write(&path, &[meta(), turn("first-turn")]);
    let mut saved = record();
    saved.scan = inspect_file(&path, &saved).unwrap().checkpoint;
    fs::rename(&path, temp.path().join("original.jsonl")).unwrap();
    write(&path, &[user("user", "요청")]);
    assert!(!inspect_file(&path, &saved).unwrap().verified);
    write(&path, &[meta(), turn("later-turn"), user("user", "요청")]);
    assert!(!inspect_file(&path, &saved).unwrap().verified);
}

#[test]
fn matched_snapshot_can_be_revalidated_after_a_temporary_mapping_hold() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    write(&path, &[meta(), turn("first-turn"), user("user", "요청")]);
    let mut saved = record();
    let first = inspect_file(&path, &saved).unwrap();
    assert!(first.verified);
    saved.scan = first.checkpoint;
    saved.state = "review_required".into();
    assert!(inspect_file(&path, &saved).unwrap().verified);
}
