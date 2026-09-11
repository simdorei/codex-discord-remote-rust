#[path = "support/installer_native.rs"]
mod fixture;
use std::fs;

fn helper(root: &std::path::Path) -> std::path::PathBuf {
    root.join("plugins/codex-discord-remote/bin/cdr-pro-helper")
}

#[test]
fn shell_staging_verification_failure_keeps_the_previous_helper_and_reports_the_error() {
    let root = tempfile::tempdir().unwrap();
    let codex = fixture::seed(root.path());
    fixture::inventories(root.path(), "normal");
    let destination = helper(root.path());
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"known-good helper").unwrap();
    // Inject a failed OS comparison only for the staged helper, not the installer
    // decisions or runtime staging. This exercises the actual publication path.
    let script = root.path().join("install.sh");
    let fault = r#"
cmp() {
  case "$*" in
    *cdr-pro-helper.install.*) echo 'injected comparison failure' >&2; return 1 ;;
    *) command cmp "$@" ;;
  esac
}
"#;
    fs::write(
        &script,
        fs::read_to_string(&script)
            .unwrap()
            .replacen("set -eu\n", &format!("set -eu\n{fault}"), 1),
    )
    .unwrap();
    let output = fixture::run(root.path(), false, &codex, |_| {});
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{error}");
    assert!(error.contains("injected comparison failure"), "{error}");
    assert!(error.contains("INSTALL_INCOMPLETE"), "{error}");
    assert_eq!(fs::read(&destination).unwrap(), b"known-good helper");
    assert_eq!(
        fs::read_dir(destination.parent().unwrap()).unwrap().count(),
        1
    );
    assert!(!root.path().join("cli-calls.jsonl").exists());
    assert!(!root.path().join(".codex_discord_runtime").exists());
}

#[test]
fn shell_publishes_the_exact_helper_and_repeated_install_has_no_staging_debris() {
    let root = tempfile::tempdir().unwrap();
    let codex = fixture::seed(root.path());
    fixture::inventories(root.path(), "normal");
    let destination = helper(root.path());
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"known-good helper").unwrap();
    let expected = fs::read(root.path().join("fixture-bin/cdr-pro-helper")).unwrap();
    for _ in 0..2 {
        let output = fixture::run(root.path(), false, &codex, |_| {});
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&destination).unwrap(), expected);
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            1
        );
    }
    assert_eq!(
        fs::read_to_string(root.path().join("cli-calls.jsonl"))
            .unwrap()
            .lines()
            .count(),
        8
    );
}
