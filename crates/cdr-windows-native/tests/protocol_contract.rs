#![cfg(windows)]
use cdr_windows_native::protocol::{ProtocolRegistration, registration};

#[test]
fn nonexistent_protocol_is_read_without_creation_and_codex_lookup_returns_metadata_only() {
    let missing = format!("cdr-test-{}", uuid::Uuid::new_v4().simple());
    assert_eq!(
        registration(&missing).unwrap(),
        ProtocolRegistration::KeyMissing
    );
    assert_eq!(
        registration(&missing).unwrap(),
        ProtocolRegistration::KeyMissing
    );
    // Do not assume Codex is installed on a CI worker; errors remain explicit.
    match registration("codex") {
        Ok(
            ProtocolRegistration::KeyMissing
            | ProtocolRegistration::UrlMarkerMissing
            | ProtocolRegistration::UrlMarkerPresent
            | ProtocolRegistration::UnexpectedMarkerType(_),
        ) => {}
        Err(error) => assert!(error.to_string().contains("Reg")),
    }
}
