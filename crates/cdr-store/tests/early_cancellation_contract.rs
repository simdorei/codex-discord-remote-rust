use cdr_store::{
    ingress::{self, IngressKind, NewIngress},
    queue,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Barrier},
};

fn seed(db: &Path, kind: IngressKind, payload: Value) {
    ingress::admit(
        db,
        &NewIngress {
            ingress_id: "request:1".into(),
            kind,
            event_id: Some(1),
            application_id: Some(2),
            channel_id: 42,
            owner_user_id: 3,
            source_message_id: Some(1),
            payload,
            target_thread_id: Some("original".into()),
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap();
}

#[test]
fn only_known_prompt_envelopes_are_eligible_before_execution() {
    for (kind, payload, eligible) in [
        (
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"p"}}}}),
            true,
        ),
        (
            IngressKind::Interaction,
            json!({"version":1,"work":{"Slash":{"name":"ask","values":{"prompt":{"String":"p"}}}}}),
            true,
        ),
        (
            IngressKind::Interaction,
            json!({"version":1,"work":{"Slash":{"name":"interview","values":{"prompt":{"String":"p"}}}}}),
            true,
        ),
        (
            IngressKind::Message,
            json!({"version":true,"plan":{"Execute":{"Ask":{"prompt":"p"}}}}),
            false,
        ),
        (
            IngressKind::Message,
            json!({"version":2,"plan":{"Execute":{"Ask":{"prompt":"p"}}}}),
            false,
        ),
        (
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":"Help"}}),
            false,
        ),
        (
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":{"New":{"prompt":"new target not created"}}}}),
            false,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        seed(&db, kind, payload);
        let before = ingress::get(&db, "request:1").unwrap().unwrap();
        let cancelled = queue::cancel_latest_pending(&db, "original", 42, 3, 2.0).unwrap();
        assert_eq!(cancelled.is_some(), eligible, "{:?}", before.payload);
        let after = ingress::get(&db, "request:1").unwrap().unwrap();
        assert_eq!(after.payload, before.payload);
        if !eligible {
            assert_eq!(after, before);
        }
    }
}

#[test]
fn ingress_execution_and_cancellation_have_one_winner() {
    for _ in 0..12 {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        seed(
            &db,
            IngressKind::Message,
            json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"p"}}}}),
        );
        let gate = Arc::new(Barrier::new(2));
        let cancelled = {
            let (path, barrier) = (db.clone(), gate.clone());
            std::thread::spawn(move || {
                barrier.wait();
                queue::cancel_latest_pending(&path, "original", 42, 3, 2.0)
                    .is_ok_and(|v| v.is_some())
            })
        };
        gate.wait();
        let started = ingress::begin_execution(&db, "request:1", "processing", None, 2.0).unwrap();
        assert_ne!(started, cancelled.join().unwrap());
        let saved = ingress::get(&db, "request:1").unwrap().unwrap();
        assert_eq!(
            saved.phase,
            if started { "processing" } else { "cancelled" }
        );
    }
}
