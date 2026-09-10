#![cfg(windows)]

use cdr_windows_native::{NativeError, SingleInstance};

#[test]
fn named_mutex_rejects_a_duplicate_and_recovers_after_owner_drop() {
    let name = format!("Local\\CodexDiscordRemoteRustTest-{}", uuid::Uuid::new_v4());
    let first = SingleInstance::acquire(&name).unwrap();
    assert!(matches!(
        SingleInstance::acquire(&name),
        Err(NativeError::AlreadyRunning)
    ));
    drop(first);
    SingleInstance::acquire(&name).unwrap();
}
