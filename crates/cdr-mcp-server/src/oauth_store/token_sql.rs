use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::{OAuthStoreError, OAuthStoreLimits, OAuthTokenRecord};

pub(super) struct TokenRow {
    client_id: String,
    scopes_json: String,
    expires_at: Option<i64>,
    resource: Option<String>,
    subject: Option<String>,
}

impl TokenRow {
    pub(super) fn with_plaintext(self, token: &str) -> Result<OAuthTokenRecord, OAuthStoreError> {
        Ok(OAuthTokenRecord {
            token: token.to_owned(),
            client_id: self.client_id,
            scopes: serde_json::from_str(&self.scopes_json)?,
            expires_at: self.expires_at,
            resource: self.resource,
            subject: self.subject,
        })
    }
}

pub(super) fn load_token_row(
    connection: &Connection,
    token: &str,
    kind: &str,
) -> Result<Option<TokenRow>, OAuthStoreError> {
    Ok(connection
        .query_row(
            "SELECT client_id, scopes_json, expires_at, resource, subject
             FROM oauth_tokens WHERE token_hash = ? AND token_kind = ?",
            params![token_hash(token), kind],
            |row| {
                Ok(TokenRow {
                    client_id: row.get(0)?,
                    scopes_json: row.get(1)?,
                    expires_at: row.get(2)?,
                    resource: row.get(3)?,
                    subject: row.get(4)?,
                })
            },
        )
        .optional()?)
}

pub(super) fn find_family(
    connection: &Connection,
    table: &str,
    token: &str,
) -> Result<Option<String>, OAuthStoreError> {
    let sql = match table {
        "oauth_tokens" => "SELECT family_id FROM oauth_tokens WHERE token_hash = ?",
        "oauth_refresh_history" => {
            "SELECT family_id FROM oauth_refresh_history WHERE token_hash = ?"
        }
        _ => {
            return Err(OAuthStoreError::Configuration(
                "unsupported OAuth family table",
            ));
        }
    };
    Ok(connection
        .query_row(sql, [token_hash(token)], |row| row.get(0))
        .optional()?)
}

pub(super) fn insert_token(
    connection: &Connection,
    token: &OAuthTokenRecord,
    kind: &str,
    family_id: &str,
) -> Result<(), OAuthStoreError> {
    let resource = (kind == "access")
        .then_some(token.resource.as_deref())
        .flatten();
    connection.execute(
        "INSERT INTO oauth_tokens(
            token_hash, token_kind, family_id, client_id, scopes_json,
            expires_at, resource, subject
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            token_hash(&token.token),
            kind,
            family_id,
            token.client_id,
            serde_json::to_string(&token.scopes)?,
            token.expires_at,
            resource,
            token.subject,
        ],
    )?;
    Ok(())
}

pub(super) fn delete_family(
    connection: &Connection,
    family_id: &str,
) -> Result<(), OAuthStoreError> {
    connection.execute("DELETE FROM oauth_tokens WHERE family_id = ?", [family_id])?;
    Ok(())
}

pub(super) fn delete_expired(connection: &Connection, now: i64) -> Result<(), OAuthStoreError> {
    connection.execute(
        "DELETE FROM oauth_tokens WHERE expires_at IS NOT NULL AND expires_at < ?",
        [now],
    )?;
    connection.execute(
        "DELETE FROM oauth_refresh_history WHERE expires_at < ?",
        [now],
    )?;
    Ok(())
}

pub(super) fn family_count(
    connection: &Connection,
    client_id: Option<&str>,
) -> Result<i64, OAuthStoreError> {
    let count = match client_id {
        Some(client_id) => connection.query_row(
            "SELECT COUNT(DISTINCT family_id) FROM oauth_tokens WHERE client_id = ?",
            [client_id],
            |row| row.get(0),
        )?,
        None => connection.query_row(
            "SELECT COUNT(DISTINCT family_id) FROM oauth_tokens",
            [],
            |row| row.get(0),
        )?,
    };
    Ok(count)
}

pub(super) fn ensure_family_room(
    connection: &Connection,
    limits: OAuthStoreLimits,
    client_id: &str,
    family_id: &str,
) -> Result<(), OAuthStoreError> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM oauth_tokens WHERE family_id = ?",
            [family_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        return Ok(());
    }
    if family_count(connection, Some(client_id))? >= limits.max_token_families_per_client {
        return Err(OAuthStoreError::TokenFamilyClientLimit);
    }
    if family_count(connection, None)? >= limits.max_token_families {
        return Err(OAuthStoreError::TokenFamilyGlobalLimit);
    }
    Ok(())
}

pub(super) fn history_at_capacity(
    connection: &Connection,
    limits: OAuthStoreLimits,
    family_id: &str,
) -> Result<bool, OAuthStoreError> {
    let family_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM oauth_refresh_history WHERE family_id = ?",
        [family_id],
        |row| row.get(0),
    )?;
    if family_count >= limits.max_refresh_history_per_family {
        return Ok(true);
    }
    let global_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM oauth_refresh_history", [], |row| {
            row.get(0)
        })?;
    Ok(global_count >= limits.max_refresh_history_global)
}

pub(super) fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
