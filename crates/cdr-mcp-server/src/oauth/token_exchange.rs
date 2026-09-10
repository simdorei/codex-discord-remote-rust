use super::model::{AuthorizationCodeRecord, token_response};
use super::util::{constant_time_matches, new_secret, now, validate_scopes, verify_pkce};
use super::{
    CodeExchangeRequest, OAuthProvider, OAuthProviderError, OAuthTokenResponse,
    RefreshExchangeRequest,
};
use crate::oauth_store::{OAuthStoreError, OAuthTokenRecord, RefreshRotationOutcome};

impl OAuthProvider {
    pub async fn exchange_code(
        &self,
        request: CodeExchangeRequest,
    ) -> Result<OAuthTokenResponse, OAuthProviderError> {
        self.authenticate_client(&request.client_id, &request.client_secret)
            .await?;
        if request.resource != self.resource_url.as_str() {
            return Err(OAuthProviderError::InvalidGrant);
        }
        let code = self.claim_valid_code(&request).await?;
        let (access, refresh) = self.new_token_pair(&code.client_id, &code.scopes)?;
        let store = self.store.clone();
        let saved_access = access.clone();
        let saved_refresh = refresh.clone();
        let family_id = new_secret(1);
        let provider = self.clone();
        let code_for_completion = code.clone();
        let task = tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                store.save_token_pair(&saved_access, &saved_refresh, &family_id)
            })
            .await
            .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))?;
            let mut state = provider.state.lock().await;
            state.exchanging.remove(&code_for_completion.code);
            if result.is_err() {
                state.codes.insert(
                    code_for_completion.code.clone(),
                    code_for_completion.clone(),
                );
            }
            result.map_err(map_store_error)
        });
        task.await
            .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))??;
        Ok(token_response(
            access,
            refresh,
            self.config.access_token_seconds,
        ))
    }

    pub async fn exchange_refresh(
        &self,
        request: RefreshExchangeRequest,
    ) -> Result<OAuthTokenResponse, OAuthProviderError> {
        self.authenticate_client(&request.client_id, &request.client_secret)
            .await?;
        if request.resource != self.resource_url.as_str() {
            return Err(OAuthProviderError::InvalidGrant);
        }
        let store = self.store.clone();
        let presented = request.refresh_token.clone();
        let existing = tokio::task::spawn_blocking(move || store.load_refresh_token(&presented))
            .await
            .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))??
            .ok_or(OAuthProviderError::InvalidGrant)?;
        if existing.client_id != request.client_id {
            return Err(OAuthProviderError::InvalidGrant);
        }
        let scopes = if request.scopes.is_empty() {
            existing.scopes.clone()
        } else {
            request.scopes
        };
        validate_scopes(&scopes)?;
        if !scopes.iter().all(|scope| existing.scopes.contains(scope)) {
            return Err(OAuthProviderError::InvalidScope);
        }
        let (access, refresh) = self.new_token_pair(&existing.client_id, &scopes)?;
        let store = self.store.clone();
        let old_refresh = existing.token;
        let next_access = access.clone();
        let next_refresh = refresh.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            store.rotate_token_pair(&old_refresh, &next_access, &next_refresh)
        })
        .await
        .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))??;
        if outcome != RefreshRotationOutcome::Rotated {
            return Err(OAuthProviderError::InvalidGrant);
        }
        Ok(token_response(
            access,
            refresh,
            self.config.access_token_seconds,
        ))
    }

    pub async fn authenticate_access(
        &self,
        token: &str,
    ) -> Result<Option<OAuthTokenRecord>, OAuthProviderError> {
        let Some(record) = self.store_access(token.to_owned()).await? else {
            return Ok(None);
        };
        let timestamp = now()?;
        if record
            .expires_at
            .is_some_and(|expires| expires <= timestamp)
            || record.resource.as_deref() != Some(self.resource_url.as_str())
        {
            return Ok(None);
        }
        Ok(Some(record))
    }

    pub async fn revoke(
        &self,
        client_id: &str,
        client_secret: &str,
        token: String,
    ) -> Result<(), OAuthProviderError> {
        self.authenticate_client(client_id, client_secret).await?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.revoke_family(&token))
            .await
            .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))??;
        Ok(())
    }

    async fn authenticate_client(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<(), OAuthProviderError> {
        let client = self
            .load_client(client_id)
            .await?
            .ok_or(OAuthProviderError::InvalidClient)?;
        if client.client_id == client_id
            && constant_time_matches(client_secret, &client.client_secret)
        {
            Ok(())
        } else {
            Err(OAuthProviderError::InvalidClient)
        }
    }

    async fn claim_valid_code(
        &self,
        request: &CodeExchangeRequest,
    ) -> Result<AuthorizationCodeRecord, OAuthProviderError> {
        let mut state = self.state.lock().await;
        state.discard_expired(now()?);
        let code = state
            .codes
            .get(&request.code)
            .filter(|code| {
                code.client_id == request.client_id
                    && code.redirect_uri == request.redirect_uri
                    && code.resource == request.resource
                    && verify_pkce(&request.code_verifier, &code.code_challenge)
            })
            .cloned()
            .ok_or(OAuthProviderError::InvalidGrant)?;
        state.codes.remove(&request.code);
        state.exchanging.insert(request.code.clone(), code.clone());
        Ok(code)
    }

    fn new_token_pair(
        &self,
        client_id: &str,
        scopes: &[String],
    ) -> Result<(OAuthTokenRecord, OAuthTokenRecord), OAuthProviderError> {
        let timestamp = now()?;
        let common = OAuthTokenRecord {
            token: String::new(),
            client_id: client_id.to_owned(),
            scopes: scopes.to_vec(),
            expires_at: None,
            resource: None,
            subject: Some("owner".to_owned()),
        };
        let access = OAuthTokenRecord {
            token: new_secret(2),
            expires_at: Some(timestamp + self.config.access_token_seconds),
            resource: Some(self.resource_url.to_string()),
            ..common.clone()
        };
        let refresh = OAuthTokenRecord {
            token: new_secret(2),
            expires_at: Some(timestamp + self.config.refresh_token_seconds),
            ..common
        };
        Ok((access, refresh))
    }
}

fn map_store_error(error: OAuthStoreError) -> OAuthProviderError {
    match error {
        OAuthStoreError::ClientLimit
        | OAuthStoreError::TokenFamilyClientLimit
        | OAuthStoreError::TokenFamilyGlobalLimit => OAuthProviderError::TemporarilyUnavailable,
        other => OAuthProviderError::Store(other),
    }
}
