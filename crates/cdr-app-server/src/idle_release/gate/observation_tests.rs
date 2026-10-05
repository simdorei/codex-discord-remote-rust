use super::TargetGate;
use std::sync::atomic::Ordering;

#[test]
fn unattributed_gap_is_sticky_before_persistence_but_ordinary_admission_is_unchanged() {
    let gate = std::sync::Arc::new(TargetGate::default());
    gate.mark_gap();
    let old = gate.gap_epoch();
    gate.hold_unattributed_gap();
    assert!(!gate.clear_gap(old));
    assert!(!gate.clear_gap(gate.gap_epoch()));
    gate.mark_gap();
    assert!(!gate.observations_verified());
    let ordinary = gate.admit("owner", 1, Some("B".into())).unwrap();
    assert!(matches!(ordinary, super::MutationAdmission::Ordinary(_)));
    assert!(!gate.observations_verified());
}

#[test]
fn new_gap_epoch_cannot_be_cleared_by_an_older_proof() {
    let gate = TargetGate::default();
    gate.mark_gap();
    let old = gate.gap_epoch();
    gate.mark_gap();
    assert!(!gate.clear_gap(old));
    assert!(!gate.observations_verified());
    assert!(gate.clear_gap(gate.gap_epoch()));
    assert!(gate.observations_verified());
}

#[test]
fn exhausted_gap_epoch_remains_dirty_without_aba() {
    let gate = TargetGate::default();
    gate.observation_gap.store(u64::MAX - 1, Ordering::Release);
    gate.mark_gap();
    assert_eq!(gate.gap_epoch(), u64::MAX);
    gate.mark_gap();
    assert!(!gate.clear_gap(u64::MAX));
    assert!(!gate.observations_verified());
}
