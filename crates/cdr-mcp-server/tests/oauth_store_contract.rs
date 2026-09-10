use std::path::{Path, PathBuf};
use std::process::Command;

use cdr_mcp_server::oauth_store::{
    OAuthStore, OAuthStoreLimits, OAuthTokenRecord, RefreshRotationOutcome,
};
use rusqlite::Connection;

const FUTURE: i64 = 4_000_000_000;

#[test]
fn tokens_are_hashed_and_survive_restart() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("oauth.sqlite3");
    let access = access("access-secret", "client-a");
    let refresh = refresh("refresh-secret", "client-a");

    OAuthStore::open(&path, OAuthStoreLimits::default())
        .expect("open store")
        .save_token_pair(&access, &refresh, "family-a")
        .expect("save token pair");

    let connection = Connection::open(&path).expect("inspect sqlite database");
    let stored = connection
        .prepare("SELECT token_hash FROM oauth_tokens ORDER BY token_kind")
        .expect("prepare token query")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query hashes")
        .collect::<Result<Vec<_>, _>>()
        .expect("read hashes");
    assert_eq!(stored.len(), 2);
    assert!(!stored.iter().any(|value| value.contains("secret")));
    drop(connection);

    let restarted = OAuthStore::open(&path, OAuthStoreLimits::default()).expect("restart store");
    assert_eq!(
        restarted
            .load_access_token("access-secret")
            .expect("load access token")
            .expect("access token exists"),
        access
    );
    assert!(
        restarted
            .load_refresh_token("refresh-secret")
            .expect("load refresh token")
            .is_some()
    );
}

#[test]
fn spent_refresh_replay_revokes_the_successor_family() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("oauth.sqlite3");
    let store = OAuthStore::open(&path, OAuthStoreLimits::default()).expect("open store");
    store
        .save_token_pair(
            &access("access-first", "client-a"),
            &refresh("refresh-first", "client-a"),
            "family-a",
        )
        .expect("save first pair");
    assert_eq!(
        store
            .rotate_token_pair(
                "refresh-first",
                &access("access-next", "client-a"),
                &refresh("refresh-next", "client-a"),
            )
            .expect("rotate pair"),
        RefreshRotationOutcome::Rotated
    );
    drop(store);

    let restarted = OAuthStore::open(&path, OAuthStoreLimits::default()).expect("restart store");
    assert_eq!(
        restarted
            .rotate_token_pair(
                "refresh-first",
                &access("access-attacker", "client-a"),
                &refresh("refresh-attacker", "client-a"),
            )
            .expect("detect replay"),
        RefreshRotationOutcome::Replayed
    );
    assert!(
        restarted
            .load_access_token("access-next")
            .expect("load successor access")
            .is_none()
    );
    assert!(
        restarted
            .load_refresh_token("refresh-next")
            .expect("load successor refresh")
            .is_none()
    );
}

#[test]
fn python_and_rust_read_each_others_oauth_rows() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("oauth.sqlite3");
    let store = OAuthStore::open(&path, OAuthStoreLimits::default()).expect("open Rust store");
    store
        .save_token_pair(
            &access("rust-access", "rust-client"),
            &refresh("rust-refresh", "rust-client"),
            "rust-family",
        )
        .expect("save Rust pair");
    drop(store);

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read_rust = r#"import anyio, sys
from pathlib import Path
from remote_mcp_server.simdorei_mcp.oauth_store import OAuthStore

async def main():
    store = OAuthStore(Path(sys.argv[1]))
    access = await store.load_access_token("rust-access")
    refresh = await store.load_refresh_token("rust-refresh")
    assert access is not None and access.client_id == "rust-client"
    assert refresh is not None and refresh.client_id == "rust-client"
    await store.close()

anyio.run(main)
"#;
    run_python(&repo, read_rust, &path);

    let write_python = r#"import anyio, sys
from pathlib import Path
from mcp.server.auth.provider import AccessToken, RefreshToken
from remote_mcp_server.simdorei_mcp.oauth_store import OAuthStore

async def main():
    store = OAuthStore(Path(sys.argv[1]))
    access = AccessToken(token="python-access", client_id="python-client", scopes=["files:read"], expires_at=4000000000, resource="https://example.test/mcp", subject="owner")
    refresh = RefreshToken(token="python-refresh", client_id="python-client", scopes=["files:read"], expires_at=4000000000, subject="owner")
    await store.save_token_pair(access, refresh, "python-family")
    await store.close()

anyio.run(main)
"#;
    run_python(&repo, write_python, &path);

    let reopened = OAuthStore::open(&path, OAuthStoreLimits::default()).expect("reopen Rust store");
    assert_eq!(
        reopened
            .load_access_token("python-access")
            .expect("load Python access")
            .expect("Python access exists")
            .client_id,
        "python-client"
    );
    assert!(
        reopened
            .load_refresh_token("python-refresh")
            .expect("load Python refresh")
            .is_some()
    );
}

fn access(token: &str, client_id: &str) -> OAuthTokenRecord {
    OAuthTokenRecord {
        token: token.to_owned(),
        client_id: client_id.to_owned(),
        scopes: vec!["files:read".to_owned()],
        expires_at: Some(FUTURE),
        resource: Some("https://example.test/mcp".to_owned()),
        subject: Some("owner".to_owned()),
    }
}

fn refresh(token: &str, client_id: &str) -> OAuthTokenRecord {
    OAuthTokenRecord {
        resource: None,
        ..access(token, client_id)
    }
}

fn run_python(repo: &Path, script: &str, database: &Path) {
    let (mut command, site_packages) = python_command(repo);
    let script = site_packages.as_ref().map_or_else(
        || script.to_owned(),
        |_| format!("import site,sys\nsite.addsitedir(sys.argv.pop(1))\n{script}"),
    );
    command.current_dir(repo).args(["-c", &script]);
    if let Some(path) = site_packages {
        command.arg(path);
    }
    let output = command
        .arg(database)
        .output()
        .expect("run Python compatibility probe");
    assert!(
        output.status.success(),
        "Python OAuth compatibility probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn python_command(repo: &Path) -> (Command, Option<PathBuf>) {
    if let Some(executable) = std::env::var_os("PYTHON_EXE").filter(|value| !value.is_empty()) {
        return (Command::new(executable), None);
    }
    if cfg!(windows) {
        let portable = repo.join(".python-portable/python.exe");
        if portable.is_file() {
            let site_packages = repo.join("remote_mcp_server/.venv/Lib/site-packages");
            return (Command::new(portable), Some(site_packages));
        }
        let mut command = Command::new("py");
        command.arg("-3");
        (command, None)
    } else {
        let local = repo.join("remote_mcp_server/.venv/bin/python");
        if local.is_file() {
            (Command::new(local), None)
        } else {
            (Command::new("python3"), None)
        }
    }
}
