use super::model::{AuthorizationCodeRecord, PendingAuthorization};
use super::util::{constant_time_matches, endpoint, new_secret, now, validate_scopes};
use super::{AuthorizationRequest, OAuthProvider, OAuthProviderError, PendingApproval};
use crate::scopes::ALL;

const AUTHORIZATION_TTL_SECONDS: i64 = 300;
const MAX_OWNER_TOKEN_ATTEMPTS: u8 = 5;

impl OAuthProvider {
    pub async fn begin_authorization(
        &self,
        request: AuthorizationRequest,
    ) -> Result<PendingApproval, OAuthProviderError> {
        if request.resource != self.resource_url.as_str() {
            return Err(OAuthProviderError::InvalidRequest(
                "Unknown protected resource.".to_owned(),
            ));
        }
        if request.code_challenge_method != "S256"
            || !(43..=128).contains(&request.code_challenge.len())
        {
            return Err(OAuthProviderError::InvalidRequest(
                "PKCE S256 is required.".to_owned(),
            ));
        }
        let client = self
            .load_client(&request.client_id)
            .await?
            .ok_or(OAuthProviderError::InvalidClient)?;
        if !client.redirect_uris.contains(&request.redirect_uri) {
            return Err(OAuthProviderError::InvalidRequest(
                "redirect_uri does not match the registered client.".to_owned(),
            ));
        }
        let scopes = if request.scopes.is_empty() {
            ALL.iter().map(ToString::to_string).collect()
        } else {
            request.scopes.clone()
        };
        validate_scopes(&scopes)?;
        let client_scopes = client.scope.split_whitespace().collect::<Vec<_>>();
        if !scopes
            .iter()
            .all(|scope| client_scopes.contains(&scope.as_str()))
        {
            return Err(OAuthProviderError::InvalidScope);
        }
        let request_id = new_secret(1);
        let mut state = self.state.lock().await;
        let timestamp = now()?;
        state.discard_expired(timestamp);
        if state.pending.len() >= self.config.pending_authorization_limit {
            return Err(OAuthProviderError::TemporarilyUnavailable);
        }
        state.pending.insert(
            request_id.clone(),
            PendingAuthorization {
                request,
                scopes,
                expires_at: timestamp + AUTHORIZATION_TTL_SECONDS,
                failed_attempts: 0,
            },
        );
        let mut approval_url = endpoint(&self.config.public_base_url, "/oauth/approve")?;
        approval_url
            .query_pairs_mut()
            .append_pair("request_id", &request_id);
        Ok(PendingApproval {
            request_id,
            approval_url,
        })
    }

    pub async fn pending_scopes(
        &self,
        request_id: &str,
    ) -> Result<Option<Vec<String>>, OAuthProviderError> {
        let mut state = self.state.lock().await;
        state.discard_expired(now()?);
        Ok(state
            .pending
            .get(request_id)
            .map(|pending| pending.scopes.clone()))
    }

    pub async fn approve(
        &self,
        request_id: &str,
        candidate_owner_token: &str,
    ) -> Result<url::Url, OAuthProviderError> {
        let mut state = self.state.lock().await;
        let timestamp = now()?;
        state.discard_expired(timestamp);
        let pending = state
            .pending
            .get(request_id)
            .cloned()
            .ok_or(OAuthProviderError::ApprovalNotFound)?;
        if !constant_time_matches(candidate_owner_token, &self.config.owner_token) {
            let failed_attempts = pending.failed_attempts + 1;
            if failed_attempts >= MAX_OWNER_TOKEN_ATTEMPTS {
                state.pending.remove(request_id);
            } else if let Some(stored) = state.pending.get_mut(request_id) {
                stored.failed_attempts = failed_attempts;
            }
            return Err(OAuthProviderError::ApprovalDenied);
        }
        let retained = state.codes.len() + state.exchanging.len();
        let client_retained = state
            .codes
            .values()
            .chain(state.exchanging.values())
            .filter(|code| code.client_id == pending.request.client_id)
            .count();
        if retained >= self.config.authorization_code_limit
            || client_retained >= self.config.authorization_code_per_client_limit
        {
            return Err(OAuthProviderError::TemporarilyUnavailable);
        }
        let code = new_secret(1);
        state.pending.remove(request_id);
        state.codes.insert(
            code.clone(),
            AuthorizationCodeRecord {
                code: code.clone(),
                client_id: pending.request.client_id,
                redirect_uri: pending.request.redirect_uri.clone(),
                code_challenge: pending.request.code_challenge,
                scopes: pending.scopes,
                resource: pending.request.resource,
                expires_at: timestamp + AUTHORIZATION_TTL_SECONDS,
            },
        );
        let mut callback = pending.request.redirect_uri.parse::<url::Url>()?;
        let issuer = self.config.public_base_url.as_str().trim_end_matches('/');
        {
            let mut query = callback.query_pairs_mut();
            query.append_pair("code", &code).append_pair("iss", issuer);
            if let Some(value) = pending.request.state {
                query.append_pair("state", &value);
            }
        }
        Ok(callback)
    }
}
