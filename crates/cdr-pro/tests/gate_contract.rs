use std::cell::RefCell;
use std::fs;

use cdr_pro::gate::{GateMode, SessionMirrorGate, gate_rollout_output};

#[test]
fn rejected_turn_requires_stable_rollout_size_before_opening() {
    let gate = SessionMirrorGate::default();
    gate.hold(Some("thread-1"));
    assert_eq!(gate.mode(Some("thread-1")), GateMode::Hold);
    gate.reject(Some("thread-1"));
    assert_eq!(gate.mode(Some("thread-1")), GateMode::Discard);
    assert!(!gate.discard_size_is_stable(Some("thread-1"), 100));
    assert!(!gate.discard_size_is_stable(Some("thread-1"), 120));
    assert!(gate.discard_size_is_stable(Some("thread-1"), 120));
    gate.finish_discard(Some("thread-1"));
    assert_eq!(gate.mode(Some("thread-1")), GateMode::Open);
}

#[test]
fn approved_or_blank_target_is_open() {
    let gate = SessionMirrorGate::default();
    gate.hold(Some("thread-1"));
    gate.approve(Some("thread-1"));
    assert_eq!(gate.mode(Some("thread-1")), GateMode::Open);
    gate.hold(Some("  "));
    assert_eq!(gate.mode(Some("  ")), GateMode::Open);
    assert_eq!(gate.mode(None), GateMode::Open);
}

#[test]
fn rejected_rollout_advances_cursor_only_after_size_is_stable() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let rollout = temp.path().join("rollout.jsonl");
    fs::write(&rollout, "rejected output").expect("write rejected rollout");
    let gate = SessionMirrorGate::default();
    gate.reject(Some("thread-1"));
    let updates = RefCell::new(Vec::new());
    let update = |thread: &str, path: &std::path::Path, size: u64| {
        updates
            .borrow_mut()
            .push((thread.to_owned(), path.to_path_buf(), size));
        Ok(())
    };

    assert!(gate_rollout_output(&gate, "thread-1", &rollout, update).expect("first gate"));
    assert!(updates.borrow().is_empty());
    assert!(gate_rollout_output(&gate, "thread-1", &rollout, update).expect("second gate"));
    assert_eq!(updates.borrow().len(), 1);
    assert_eq!(updates.borrow()[0].2, "rejected output".len() as u64);
    assert_eq!(gate.mode(Some("thread-1")), GateMode::Open);
}
