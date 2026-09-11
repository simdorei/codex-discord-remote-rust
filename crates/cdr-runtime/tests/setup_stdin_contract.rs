use std::io::Write;
use std::process::{Command, Stdio};

fn invoke(payload: &[u8], flags: &[&str]) -> (tempfile::TempDir, std::process::Output) {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "setup-discord", "--input-lines", "--repo-root"])
        .arg(root.path())
        .args(flags)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload).unwrap();
    let output = child.wait_with_output().unwrap();
    (root, output)
}

#[test]
fn shell_setup_input_needs_no_json_escaping_and_never_echoes_token() {
    let (root, output) = invoke("synthetic-토큰\"\\=\nbad-channel\n".as_bytes(), &[]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("Discord ID must contain digits only"),
        "{error}"
    );
    assert!(!error.contains("synthetic-"));
    assert!(output.stdout.is_empty());
    assert!(!root.path().join(".env").exists());
}

#[test]
fn shell_setup_rejects_malformed_input_or_conflicting_modes_without_mutation() {
    for (payload, flags, expected) in [
        (b"only-one-line\n".as_slice(), vec![], "exactly two lines"),
        (
            b"token\n123\nextra\n".as_slice(),
            vec![],
            "exactly two lines",
        ),
        (b"\xff\n123\n".as_slice(), vec![], "UTF-8"),
        (
            b"token\n123\n".as_slice(),
            vec!["--input-stdin"],
            "cannot be combined",
        ),
        (
            b"token\n123\n".as_slice(),
            vec!["--dry-run"],
            "cannot be combined",
        ),
    ] {
        let (root, output) = invoke(payload, &flags);
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(expected), "{error}");
        assert!(!root.path().join(".env").exists());
    }
}
