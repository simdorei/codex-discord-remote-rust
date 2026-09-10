use rusqlite::Connection;

use super::OAuthStoreError;

pub(super) fn initialize(connection: &Connection) -> Result<(), OAuthStoreError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS oauth_clients (
            client_id TEXT PRIMARY KEY,
            payload TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS oauth_tokens (
            token_hash TEXT PRIMARY KEY,
            token_kind TEXT NOT NULL,
            family_id TEXT NOT NULL,
            client_id TEXT NOT NULL,
            scopes_json TEXT NOT NULL,
            expires_at INTEGER,
            resource TEXT,
            subject TEXT
        );
        CREATE INDEX IF NOT EXISTS oauth_tokens_family
            ON oauth_tokens(family_id);
        CREATE TABLE IF NOT EXISTS oauth_refresh_history (
            token_hash TEXT PRIMARY KEY,
            family_id TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            used_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS oauth_refresh_history_family
            ON oauth_refresh_history(family_id);
        CREATE INDEX IF NOT EXISTS oauth_refresh_history_expiry
            ON oauth_refresh_history(expires_at);",
    )?;
    let mut columns = connection.prepare("PRAGMA table_info(oauth_clients)")?;
    let has_created_at = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|column| column == "created_at");
    drop(columns);
    if !has_created_at {
        connection.execute(
            "ALTER TABLE oauth_clients ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}
