#![cfg(windows)]

use std::fs;

use cdr_windows_native::{atomic_replace, protect_current_user, unprotect_current_user};

#[test]
fn dpapi_round_trip_does_not_leave_plaintext_in_ciphertext() {
    let plaintext = b"restart-project-scope-must-not-be-plaintext";

    let ciphertext = protect_current_user(plaintext).expect("DPAPI protection should succeed");

    assert!(!ciphertext.is_empty());
    assert!(
        !ciphertext
            .windows(plaintext.len())
            .any(|window| window == plaintext)
    );
    assert_eq!(
        unprotect_current_user(&ciphertext).expect("DPAPI unprotection should succeed"),
        plaintext
    );
}

#[test]
fn dpapi_round_trips_empty_plaintext() {
    let ciphertext = protect_current_user(&[]).expect("DPAPI protection should succeed");
    assert_eq!(
        unprotect_current_user(&ciphertext).expect("DPAPI unprotection should succeed"),
        Vec::<u8>::new()
    );
}

#[test]
fn atomic_replace_overwrites_an_existing_destination() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let source = directory.path().join("handoff.tmp");
    let destination = directory.path().join("handoff.json");
    fs::write(&source, b"new").expect("write source");
    fs::write(&destination, b"old").expect("write destination");

    atomic_replace(&source, &destination).expect("atomic replacement should succeed");

    assert_eq!(fs::read(&destination).expect("read destination"), b"new");
    assert!(!source.exists());
}
