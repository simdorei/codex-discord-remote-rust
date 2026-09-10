use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::token_sql::{
    delete_expired, delete_family, ensure_family_room, family_count, find_family,
    history_at_capacity, insert_token, load_token_row, token_hash,
};
use super::{
    OAuthStore, OAuthStoreError, OAuthTokenRecord, RefreshRotationOutcome, unix_timestamp,
};

impl OAuthStore {
    pub fn save_token_pair(
        &self,
        access: &OAuthTokenRecord,
        refresh: &OAuthTokenRecord,
        family_id: &str,
    ) -> Result<(), OAuthStoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        delete_expired(&transaction, unix_timestamp()?)?;
        ensure_family_room(&transaction, self.limits, &access.client_id, family_id)?;
        insert_token(&transaction, access, "access", family_id)?;
        insert_token(&transaction, refresh, "refresh", family_id)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn rotate_token_pair(
        &self,
        old_refresh_token: &str,
        access: &OAuthTokenRecord,
        refresh: &OAuthTokenRecord,
    ) -> Result<RefreshRotationOutcome, OAuthStoreError> {
        let successor_expiry = refresh.expires_at.ok_or(OAuthStoreError::Configuration(
            "Rotated refresh tokens must expire.",
        ))?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = unix_timestamp()?;
        delete_expired(&transaction, now)?;
        let old_hash = token_hash(old_refresh_token);
        let live_family = transaction
            .query_row(
                "SELECT family_id FROM oauth_tokens
                 WHERE token_hash = ? AND token_kind = 'refresh'",
                [&old_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(family_id) = live_family else {
            let replay_family = transaction
                .query_row(
                    "SELECT family_id FROM oauth_refresh_history WHERE token_hash = ?",
                    [&old_hash],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let outcome = if let Some(family_id) = replay_family {
                delete_family(&transaction, &family_id)?;
                RefreshRotationOutcome::Replayed
            } else {
                RefreshRotationOutcome::Missing
            };
            transaction.commit()?;
            return Ok(outcome);
        };
        if history_at_capacity(&transaction, self.limits, &family_id)? {
            delete_family(&transaction, &family_id)?;
            transaction.commit()?;
            return Ok(RefreshRotationOutcome::HistoryExhausted);
        }
        transaction.execute(
            "UPDATE oauth_refresh_history SET expires_at = ? WHERE family_id = ?",
            params![successor_expiry, family_id],
        )?;
        transaction.execute(
            "INSERT INTO oauth_refresh_history(token_hash, family_id, expires_at, used_at)
             VALUES (?, ?, ?, ?)",
            params![old_hash, family_id, successor_expiry, now],
        )?;
        delete_family(&transaction, &family_id)?;
        insert_token(&transaction, access, "access", &family_id)?;
        insert_token(&transaction, refresh, "refresh", &family_id)?;
        transaction.commit()?;
        Ok(RefreshRotationOutcome::Rotated)
    }

    pub fn load_access_token(
        &self,
        token: &str,
    ) -> Result<Option<OAuthTokenRecord>, OAuthStoreError> {
        let connection = self.connection()?;
        load_token_row(&connection, token, "access")?
            .map(|row| row.with_plaintext(token))
            .transpose()
    }

    pub fn load_refresh_token(
        &self,
        token: &str,
    ) -> Result<Option<OAuthTokenRecord>, OAuthStoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        delete_expired(&transaction, unix_timestamp()?)?;
        let live = load_token_row(&transaction, token, "refresh")?;
        if let Some(row) = live {
            let record = row.with_plaintext(token)?;
            transaction.commit()?;
            return Ok(Some(record));
        }
        if let Some(family_id) = find_family(&transaction, "oauth_refresh_history", token)? {
            delete_family(&transaction, &family_id)?;
        }
        transaction.commit()?;
        Ok(None)
    }

    pub fn revoke_family(&self, token: &str) -> Result<(), OAuthStoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        delete_expired(&transaction, unix_timestamp()?)?;
        let family_id = find_family(&transaction, "oauth_tokens", token)?.or(find_family(
            &transaction,
            "oauth_refresh_history",
            token,
        )?);
        if let Some(family_id) = family_id {
            delete_family(&transaction, &family_id)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn count_token_families(&self, client_id: Option<&str>) -> Result<i64, OAuthStoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        delete_expired(&transaction, unix_timestamp()?)?;
        let count = family_count(&transaction, client_id)?;
        transaction.commit()?;
        Ok(count)
    }
}
