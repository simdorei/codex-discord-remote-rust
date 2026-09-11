use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::json;

fn admin(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .arg("--admin")
        .args(args)
        .arg("--repo-root")
        .arg(root)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn pyfree_setup_dry_run_needs_no_interpreter_token_or_network() {
    let root = tempfile::tempdir().unwrap();
    let output = admin(
        root.path(),
        &["setup-discord", "--dry-run", "--bot-id", "42"],
    );
    let text = success(&output);
    assert!(
        text.contains("client_id=42&scope=bot%20applications.commands&permissions=328565115968")
    );
    assert!(text.contains("no token was requested"));
    assert!(!text.contains("DISCORD_BOT_TOKEN="));
    assert!(!root.path().join(".env").exists());
}

#[test]
fn pyfree_backup_needs_neither_a_discord_token_nor_codex() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("discord_mirror.sqlite");
    let connection = cdr_store::schema::open_initialized(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE preservation(value TEXT); INSERT INTO preservation VALUES ('한글');",
        )
        .unwrap();
    let output = admin(root.path(), &["backup-store"]);
    let text = success(&output);
    let backup = text.trim().strip_prefix("backup_created path=").unwrap();
    let restored = rusqlite::Connection::open(backup).unwrap();
    let value: String = restored
        .query_row("SELECT value FROM preservation", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "한글");
}

#[test]
fn pyfree_missing_database_reports_error_without_creating_empty_store() {
    let root = tempfile::tempdir().unwrap();
    let output = admin(root.path(), &["backup-store"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("store database was not found")
    );
    assert!(!root.path().join("discord_mirror.sqlite").exists());
}

#[test]
fn pyfree_configure_install_preserves_user_values_and_utf8() {
    let root = tempfile::tempdir().unwrap();
    let env_path = root.path().join(".env");
    fs::write(
        &env_path,
        "# 사용자 설정\r\nDISCORD_BOT_TOKEN=fake-secret=tail\r\nCODEX_EXE=keep-explicit\r\n",
    )
    .unwrap();
    let output = admin(
        root.path(),
        &["configure-install", "--codex-home", "C:/사용자/코덱스"],
    );
    let text = success(&output);
    assert!(!text.contains("fake-secret"));
    assert_eq!(
        fs::read_to_string(env_path).unwrap(),
        "# 사용자 설정\r\nDISCORD_BOT_TOKEN=fake-secret=tail\r\nCODEX_EXE=keep-explicit\r\nCODEX_HOME=C:/사용자/코덱스\r\n"
    );
}

#[test]
fn pyfree_configure_install_rejects_line_injection_without_partial_write() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".env");
    fs::write(&path, "UNCHANGED=yes\n").unwrap();
    let output = admin(
        root.path(),
        &[
            "configure-install",
            "--codex-home",
            "valid",
            "--codex-exe",
            "exe\nINJECTED=true",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("cannot contain a newline")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "UNCHANGED=yes\n");
}

#[test]
fn pyfree_unknown_admin_command_is_not_a_bot_start_or_fallback() {
    let root = tempfile::tempdir().unwrap();
    let output = admin(root.path(), &["not-a-command"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown admin command")
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

fn inventory(root: &Path, installed: &serde_json::Value, marketplace_root: &Path) {
    fs::write(
        root.join("marketplaces.json"),
        json!({"marketplaces":[{"name":"codex-discord-remote","root":marketplace_root}]})
            .to_string(),
    )
    .unwrap();
    fs::write(
        root.join("plugins.json"),
        json!({"installed":installed}).to_string(),
    )
    .unwrap();
    fs::write(
        root.join("manifest.json"),
        json!({"version":"0.1.0+verified"}).to_string(),
    )
    .unwrap();
}

fn plugin() -> serde_json::Value {
    json!({"pluginId":"codex-discord-remote@codex-discord-remote","installed":true,"enabled":true,"version":"0.1.0+verified"})
}

fn verify(root: &Path) -> Output {
    admin(
        root,
        &[
            "verify-plugin-inventory",
            "--marketplace-inventory",
            "marketplaces.json",
            "--plugin-inventory",
            "plugins.json",
            "--plugin-manifest",
            "manifest.json",
        ],
    )
}

#[test]
fn pyfree_inventory_verifies_exact_enabled_version_and_source_root() {
    let root = tempfile::tempdir().unwrap();
    inventory(root.path(), &json!([plugin()]), root.path());
    assert!(success(&verify(root.path())).contains("0.1.0+verified"));
    let wrong_root = tempfile::tempdir().unwrap();
    inventory(root.path(), &json!([plugin()]), wrong_root.path());
    let output = verify(root.path());
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("wrong repository")
    );
}

#[test]
fn pyfree_inventory_rejects_disabled_stale_duplicate_and_malformed_records() {
    let root = tempfile::tempdir().unwrap();
    let mut disabled = plugin();
    disabled["enabled"] = json!(false);
    let mut stale = plugin();
    stale["version"] = json!("old");
    for records in [
        json!([disabled]),
        json!([stale]),
        json!([plugin(), plugin()]),
        json!([null]),
        json!([]),
    ] {
        inventory(root.path(), &records, root.path());
        let output = verify(root.path());
        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("INSTALL_INCOMPLETE")
        );
    }
}
