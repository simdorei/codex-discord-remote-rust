use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::token_sql::delete_expired;
use super::{OAuthStore, OAuthStoreError, unix_timestamp};

impl OAuthStore {
    pub fn get_client_payload(&self, client_id: &str) -> Result<Option<String>, OAuthStoreError> {
        let connection = self.connection()?;
        Ok(connection
            .query_row(
                "SELECT payload FROM oauth_clients WHERE client_id = ?",
                [client_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn save_client_payload(
        &self,
        client_id: &str,
        payload: &str,
    ) -> Result<(), OAuthStoreError> {
        let _: serde_json::Value = serde_json::from_str(payload)?;
        if client_id.is_empty() {
            return Err(OAuthStoreError::Configuration(
                "OAuth client_id is required.",
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = unix_timestamp()?;
        delete_expired(&transaction, now)?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM oauth_clients WHERE client_id = ?",
                [client_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            make_client_room(&transaction, self.limits.max_clients, now)?;
        }
        transaction.execute(
            "INSERT INTO oauth_clients(client_id, payload, created_at)
             VALUES (?, ?, ?)
             ON CONFLICT(client_id) DO UPDATE SET payload = excluded.payload",
            params![client_id, payload, now],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn make_client_room(
    connection: &rusqlite::Connection,
    max_clients: i64,
    now: i64,
) -> Result<(), OAuthStoreError> {
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM oauth_clients", [], |row| row.get(0))?;
    let required = count - max_clients + 1;
    if required <= 0 {
        return Ok(());
    }
    let mut query = connection.prepare(
        "SELECT client.client_id
         FROM oauth_clients AS client
         WHERE NOT EXISTS (
             SELECT 1 FROM oauth_tokens AS token
             WHERE token.client_id = client.client_id
               AND (token.expires_at IS NULL OR token.expires_at >= ?)
         )
         ORDER BY client.created_at ASC, client.rowid ASC
         LIMIT ?",
    )?;
    let inactive = query
        .query_map(params![now, required], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(query);
    if i64::try_from(inactive.len()).unwrap_or(i64::MAX) < required {
        return Err(OAuthStoreError::ClientLimit);
    }
    for client_id in inactive {
        connection.execute("DELETE FROM oauth_clients WHERE client_id = ?", [client_id])?;
    }
    Ok(())
}
