#![cfg(windows)]
use std::{path::Path, process::Command};
#[test]
fn restart_qa_uses_verified_rust_sender_and_preserves_uncertain_delivery() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for variant in ["success", "failure", "changed"] {
        let root = tempfile::tempdir().unwrap();
        let output=Command::new("powershell.exe").args(["-NoProfile","-Command",r"
$ErrorActionPreference='Stop'
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_SOURCE 'crates/cdr-runtime/tests/fixtures/restart/notification.ps1'),[Text.Encoding]::UTF8))) -Variant $env:CDR_VARIANT
exit 0
"]).env("CDR_SOURCE",&source).env("CDR_FIXTURE_ROOT",root.path()).env("CDR_VARIANT",variant).output().unwrap();
        assert!(
            output.status.success(),
            "{variant}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
