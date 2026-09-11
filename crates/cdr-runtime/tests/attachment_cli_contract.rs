use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(root: &Path, args: &[&str], token: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cdr-runtime"));
    command
        .env_clear()
        .current_dir(root)
        .args(["--admin", "send-attachment", "--repo-root"])
        .arg(root)
        .args(args);
    if token {
        command.env("DISCORD_BOT_TOKEN", "synthetic-secret");
    }
    command.output().unwrap()
}
fn failure(output: Output, message: &str) {
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains(message), "{error}");
    assert!(!error.contains("synthetic-secret"));
    assert!(output.stdout.is_empty());
}
#[test]
fn missing_token_is_explicit_before_file_access() {
    let root = tempfile::tempdir().unwrap();
    failure(
        run(root.path(), &["--channel-id", "123", "missing.txt"], false),
        "DISCORD_BOT_TOKEN is missing",
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn missing_files_fail_before_network() {
    let root = tempfile::tempdir().unwrap();
    failure(
        run(root.path(), &["--channel-id", "123", "missing.txt"], true),
        "missing.txt",
    );
}
#[test]
fn ambiguous_or_absent_targets_are_rejected_before_any_send() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec![
            "--channel-id",
            "123",
            "--work-thread",
            "thread-1",
            "note.txt",
        ],
        vec!["note.txt"],
    ] {
        failure(run(root.path(), &args, true), "exactly one target");
    }
}
#[test]
fn a_target_without_files_or_with_invalid_channel_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    failure(
        run(root.path(), &["--thread-ref", "repo:2"], true),
        "at least one attachment",
    );
    fs::write(root.path().join("note.txt"), "한글 내용").unwrap();
    failure(
        run(
            root.path(),
            &["--channel-id", "not-an-id", "note.txt"],
            true,
        ),
        "Discord ID",
    );
}
