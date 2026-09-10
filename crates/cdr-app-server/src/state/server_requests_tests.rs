use serde_json::json;
use std::sync::{Arc, Barrier, Mutex};

use super::*;

fn occurrence(value: u128) -> ServerRequestOccurrence {
    ServerRequestOccurrence::from_bytes(value.to_be_bytes())
}

fn request(id: RequestId, token: u128, command: &str) -> ServerRequest {
    ServerRequest {
        id,
        occurrence: occurrence(token),
        method: "item/commandExecution/requestApproval".to_owned(),
        params: json!({"threadId": "thread-a", "command": command}),
    }
}

#[test]
fn response_claim_detaches_old_id_and_exact_resolution_preserves_immediate_reuse() {
    let mut state = ServerRequestState::default();
    let old = request(RequestId::String("same".to_owned()), 1, "old");
    let new = request(RequestId::String("same".to_owned()), 2, "new");
    state.record(old.clone()).expect("record old");
    state
        .begin_response(&old.id, old.occurrence)
        .expect("claim old");

    assert!(state.pending(None).is_empty());
    state.record(new.clone()).expect("record immediate reuse");
    assert!(state.pending(None).is_empty());
    assert_eq!(state.unsettled(None), vec![old.clone(), new.clone()]);

    let promoted = state
        .resolve(&old.id, old.occurrence)
        .expect("resolve exact old occurrence");
    assert_eq!(promoted, Some(new.clone()));
    assert_eq!(state.pending(None), vec![new.clone()]);
    assert_eq!(state.unsettled(None), vec![new]);
}

#[test]
fn responding_identical_redelivery_is_suppressed_but_post_resolve_reuse_is_fresh() {
    let mut state = ServerRequestState::default();
    let original = request(RequestId::String("same".to_owned()), 1, "same");
    let redelivery = request(RequestId::String("same".to_owned()), 2, "same");
    state.record(original.clone()).expect("record original");
    state
        .begin_response(&original.id, original.occurrence)
        .expect("claim original");

    assert!(matches!(
        state.record(redelivery),
        Ok(ServerRequestRecordOutcome::Duplicate)
    ));
    assert_eq!(state.unsettled(None), vec![original.clone()]);
    assert_eq!(
        state
            .resolve(&original.id, original.occurrence)
            .expect("resolve original"),
        None
    );

    let reused = request(RequestId::String("same".to_owned()), 3, "same");
    let outcome = state.record(reused.clone()).expect("record reuse");
    let ServerRequestRecordOutcome::Broadcast(broadcast) = outcome else {
        panic!("post-resolve reuse must be broadcast");
    };
    assert_ne!(broadcast.occurrence, original.occurrence);
    assert_eq!(state.pending(None), vec![reused]);
}

#[test]
fn double_response_is_claimed_once_and_failure_becomes_indeterminate() {
    let mut state = ServerRequestState::default();
    let request = request(RequestId::Integer(7), 7, "once");
    state.record(request.clone()).expect("record");
    state
        .begin_response(&request.id, request.occurrence)
        .expect("first claim");
    assert!(matches!(
        state.begin_response(&request.id, request.occurrence),
        Err(AppServerError::ServerRequestResponseInFlight { .. })
    ));

    state.mark_indeterminate(&request.id, request.occurrence);
    let redelivery = ServerRequest {
        occurrence: occurrence(8),
        ..request.clone()
    };
    assert!(matches!(
        state.record(redelivery.clone()),
        Ok(ServerRequestRecordOutcome::Duplicate)
    ));
    assert!(matches!(
        state.begin_response(&request.id, request.occurrence),
        Err(AppServerError::ServerRequestResponseIndeterminate { .. })
    ));
    assert!(matches!(
        state.begin_response(&redelivery.id, redelivery.occurrence),
        Err(AppServerError::StaleServerRequest { .. })
    ));
    assert!(state.pending(None).is_empty());
    assert_eq!(state.unsettled(None), vec![request]);
    assert!(state.has_unsettled());
}

#[test]
fn simultaneous_response_claims_have_exactly_one_winner() {
    let request = request(RequestId::Integer(8), 8, "once");
    let mut initial = ServerRequestState::default();
    initial.record(request.clone()).expect("record");
    let state = Arc::new(Mutex::new(initial));
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        let request = request.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            state
                .lock()
                .expect("state lock")
                .begin_response(&request.id, request.occurrence)
        }));
    }
    barrier.wait();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("claim thread"))
        .collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(
                result,
                Err(AppServerError::ServerRequestResponseInFlight { .. })
            ))
            .count(),
        1
    );
}

#[test]
fn one_deferred_candidate_dedupes_identical_and_rejects_different_payload() {
    let mut state = ServerRequestState::default();
    let old = request(RequestId::String("same".to_owned()), 20, "old");
    let candidate = request(RequestId::String("same".to_owned()), 21, "candidate");
    let duplicate = request(RequestId::String("same".to_owned()), 22, "candidate");
    let conflict = request(RequestId::String("same".to_owned()), 23, "different");
    state.record(old.clone()).expect("record old");
    state
        .begin_response(&old.id, old.occurrence)
        .expect("claim old");
    assert!(matches!(
        state.record(candidate.clone()),
        Ok(ServerRequestRecordOutcome::Deferred)
    ));
    assert!(matches!(
        state.record(duplicate),
        Ok(ServerRequestRecordOutcome::Deferred)
    ));
    assert!(matches!(
        state.record(conflict),
        Err(ServerRequestRecordError::Conflict { .. })
    ));
    assert_eq!(state.unsettled(None), vec![old.clone(), candidate.clone()]);

    assert_eq!(
        state
            .resolve(&old.id, old.occurrence)
            .expect("promote candidate"),
        Some(candidate.clone())
    );
    assert_eq!(state.pending(None), vec![candidate]);
}

#[test]
fn stale_occurrence_cannot_claim_newer_same_id() {
    let mut state = ServerRequestState::default();
    let old = request(RequestId::String("same".to_owned()), 10, "old");
    let new = request(RequestId::String("same".to_owned()), 11, "new");
    state.record(old.clone()).expect("record old");
    state
        .begin_response(&old.id, old.occurrence)
        .expect("claim old");
    assert_eq!(
        state.resolve(&old.id, old.occurrence).expect("resolve old"),
        None
    );
    state.record(new.clone()).expect("record new");

    assert!(matches!(
        state.begin_response(&old.id, old.occurrence),
        Err(AppServerError::StaleServerRequest { .. })
    ));
    assert_eq!(state.pending(None), vec![new]);
}

#[test]
fn saturation_rejects_501st_until_exact_occurrence_resolves() {
    let mut state = ServerRequestState::default();
    for id in 0_u16..500 {
        state
            .record(request(
                RequestId::Integer(i64::from(id)),
                u128::from(id) + 1,
                "held",
            ))
            .expect("fill capacity");
    }
    let oldest = request(RequestId::Integer(0), 1, "held");
    state
        .begin_response(&oldest.id, oldest.occurrence)
        .expect("claim oldest");
    let overflow = request(RequestId::Integer(500), 501, "overflow");
    assert!(matches!(
        state.record(overflow.clone()),
        Err(ServerRequestRecordError::Saturated { .. })
    ));
    assert_eq!(state.unsettled(None).len(), 500);

    assert_eq!(
        state
            .resolve(&oldest.id, oldest.occurrence)
            .expect("free exact slot"),
        None
    );
    state.record(overflow.clone()).expect("reuse freed slot");
    assert_eq!(state.unsettled(None).len(), 500);
    assert!(state.pending(None).contains(&overflow));
}
