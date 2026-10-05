use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

fn probe(root: &Path, database: &Path, env: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .args(["--admin", "check-recovery-compatibility", "--database"])
        .arg(database)
        .arg("--env")
        .arg(env)
        .env_remove("CODEX_DISCORD_ROOT")
        .env_remove("CODEX_DISCORD_MIRROR_DB")
        .current_dir(root)
        .output()
        .unwrap()
}

fn initialized(path: &Path) -> Vec<u8> {
    drop(cdr_store::schema::open_initialized(path).unwrap());
    std::fs::read(path).unwrap()
}

#[test]
fn explicit_environment_resolves_relative_database_like_actual_startup() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let before = initialized(&db);
    let env = temp.path().join("selected.env");
    std::fs::write(&env, "CODEX_DISCORD_MIRROR_DB=store.sqlite\n").unwrap();
    let output = probe(temp.path(), &db, &env);
    assert!(output.status.success(), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["configured_environment_verified"], true);
    assert_eq!(result["admission_authorized"], false);
    assert_eq!(result["recovery_authorized"], false);
    assert_eq!(std::fs::read(&db).unwrap(), before);
}

#[test]
fn configured_root_fallback_uses_the_shared_startup_resolver() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("configured");
    std::fs::create_dir(&child).unwrap();
    let db = child.join("discord_mirror.sqlite");
    let before = initialized(&db);
    let env = temp.path().join("selected.env");
    std::fs::write(&env, "CODEX_DISCORD_ROOT=configured\n").unwrap();
    let output = probe(temp.path(), &db, &env);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(db).unwrap(), before);
}

#[test]
fn checking_another_compatible_database_does_not_authorize_configured_startup() {
    let temp = tempfile::tempdir().unwrap();
    let checked = temp.path().join("checked.sqlite");
    let configured = temp.path().join("configured.sqlite");
    let before_checked = initialized(&checked);
    let before_configured = initialized(&configured);
    let env = temp.path().join("selected.env");
    std::fs::write(&env, "CODEX_DISCORD_MIRROR_DB=configured.sqlite\n").unwrap();
    let output = probe(temp.path(), &checked, &env);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("configured database differs"),
        "{output:?}"
    );
    assert_eq!(std::fs::read(checked).unwrap(), before_checked);
    assert_eq!(std::fs::read(configured).unwrap(), before_configured);
}

#[test]
fn missing_explicit_environment_is_not_assumed_to_be_default_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let before = initialized(&db);
    let output = probe(temp.path(), &db, &temp.path().join("missing.env"));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("explicit environment"),
        "{output:?}"
    );
    assert_eq!(std::fs::read(db).unwrap(), before);
}
