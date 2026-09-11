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
fn legacy_oauth_rows_and_rust_rows_share_the_frozen_storage_contract() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("oauth.sqlite3");
    let legacy = Connection::open(&path).unwrap();
    legacy
        .execute_batch(include_str!(
            "../../../fixtures/parity/legacy_oauth_store.sql"
        ))
        .unwrap();
    drop(legacy);

    let store = OAuthStore::open(&path, OAuthStoreLimits::default()).unwrap();
    assert_eq!(
        store.load_access_token("python-access").unwrap().unwrap(),
        access("python-access", "python-client")
    );
    assert_eq!(
        store.load_refresh_token("python-refresh").unwrap().unwrap(),
        refresh("python-refresh", "python-client")
    );
    store
        .save_token_pair(
            &access("rust-access", "rust-client"),
            &refresh("rust-refresh", "rust-client"),
            "rust-family",
        )
        .unwrap();
    drop(store);

    // Independent legacy reader contract: look up SHA-256, deserialize scopes, preserve nullable fields.
    let legacy = Connection::open(&path).unwrap();
    for (hash, kind, resource) in [
        (
            "5103330be22f14d4941af0a9fe741da658d6270fd2a874f256a34a1517a413a0",
            "access",
            Some("https://example.test/mcp"),
        ),
        (
            "a790161e1a5695e28780e1647721bb1aee3badaa8e5b50c38a49aab170b1021f",
            "refresh",
            None,
        ),
    ] {
        let row = legacy.query_row(
            "SELECT token_kind,family_id,client_id,scopes_json,expires_at,resource,subject FROM oauth_tokens WHERE token_hash=?",
            [hash], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
                r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?)),
        ).unwrap();
        assert_eq!(row.0, kind);
        assert_eq!(row.1, "rust-family");
        assert_eq!(row.2, "rust-client");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&row.3).unwrap(),
            ["files:read"]
        );
        assert_eq!(row.4, FUTURE);
        assert_eq!(row.5.as_deref(), resource);
        assert_eq!(row.6.as_deref(), Some("owner"));
    }
    drop(legacy);
    let reopened = OAuthStore::open(&path, OAuthStoreLimits::default()).unwrap();
    assert_eq!(
        reopened
            .load_access_token("python-access")
            .unwrap()
            .unwrap()
            .client_id,
        "python-client"
    );
    assert!(
        reopened
            .load_refresh_token("python-refresh")
            .unwrap()
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
