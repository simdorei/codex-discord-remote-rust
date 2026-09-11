use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

const SCOPE: &str = "codex-pro-0123456789abcdef01234567";
const URL: &str = "https://chatgpt.com/c/01234567-89ab-cdef-0123-456789abcdef";
const NEXT: &str = "https://chatgpt.com/c/fedcba98-7654-3210-fedc-ba9876543210";

fn command(path: &Path, action: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cdr-pro-helper"))
        .args(["conversation", action, "--scope", SCOPE])
        .args(args)
        .env_clear()
        .env("SIMDOREI_PRO_CONVERSATION_DB", path)
        .output()
        .unwrap()
}

fn run(path: &Path, action: &str, args: &[&str]) -> Value {
    let output = command(path, action, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn save_initial(path: &Path) {
    let lease = run(path, "acquire", &[]);
    assert_eq!(
        run(
            path,
            "set",
            &[
                "--url",
                URL,
                "--lease-token",
                lease["lease_token"].as_str().unwrap()
            ]
        ),
        json!({"status":"saved"})
    );
}

#[test]
fn persisted_lease_and_url_survive_processes_and_duplicate_acquire_is_busy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("conversations.sqlite3");
    let lease = run(&path, "acquire", &[]);
    assert_eq!(lease["status"], "acquired");
    assert!(lease["lease_token"].as_str().unwrap().starts_with("lease_"));
    assert_eq!(run(&path, "acquire", &[]), json!({"status":"busy"}));
    run(
        &path,
        "set",
        &[
            "--url",
            URL,
            "--lease-token",
            lease["lease_token"].as_str().unwrap(),
        ],
    );
    assert_eq!(
        run(&path, "acquire", &[]),
        json!({"status":"found","url":URL})
    );
}

#[test]
fn two_processes_cannot_both_own_one_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("conversations.sqlite3");
    save_initial(&path);
    let (one, two) = std::thread::scope(|scope| {
        let first = scope.spawn(|| run(&path, "restart", &["--failed-url", URL]));
        let second = scope.spawn(|| run(&path, "restart", &["--failed-url", URL]));
        (first.join().unwrap(), second.join().unwrap())
    });
    let mut states = [
        one["status"].as_str().unwrap(),
        two["status"].as_str().unwrap(),
    ];
    states.sort_unstable();
    assert_eq!(states, ["acquired", "busy"]);
    let owner = if one["status"] == "acquired" {
        one
    } else {
        two
    };
    run(
        &path,
        "set",
        &[
            "--url",
            NEXT,
            "--lease-token",
            owner["lease_token"].as_str().unwrap(),
        ],
    );
    assert_eq!(
        run(&path, "restart", &["--failed-url", URL]),
        json!({"status":"superseded","url":NEXT})
    );
    assert_eq!(
        run(&path, "restart", &["--failed-url", NEXT]),
        json!({"status":"exhausted","url":NEXT})
    );
    assert_eq!(
        run(&path, "delete", &[]),
        json!({"status":"protected","url":NEXT})
    );
    assert_eq!(
        run(&path, "complete-restart", &["--url", NEXT]),
        json!({"status":"completed"})
    );
    assert_eq!(run(&path, "delete", &[]), json!({"status":"deleted"}));
}

#[test]
fn invalid_url_never_saves_or_leaks_the_lease() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("conversations.sqlite3");
    let owner = run(&path, "acquire", &[]);
    let token = owner["lease_token"].as_str().unwrap();
    for url in [
        "http://chatgpt.com/c/x",
        "https://evil.example/c/x",
        "https://user:secret@chatgpt.com/c/x",
        "https://chatgpt.com/",
    ] {
        let output = command(&path, "set", &["--url", url, "--lease-token", token]);
        assert_eq!(output.status.code(), Some(2));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(token));
    }
    assert_eq!(run(&path, "status", &[]), json!({"status":"busy"}));
}
