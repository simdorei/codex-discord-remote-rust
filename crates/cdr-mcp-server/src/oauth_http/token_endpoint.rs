use super::forms::TokenForm;
use crate::oauth::{
    CodeExchangeRequest, OAuthProvider, OAuthProviderError, OAuthTokenResponse,
    RefreshExchangeRequest,
};

pub(super) async fn exchange(
    provider: &OAuthProvider,
    form: TokenForm,
) -> Result<OAuthTokenResponse, OAuthProviderError> {
    let resource = form
        .resource
        .clone()
        .unwrap_or_else(|| provider.resource_url().to_string());
    match form.grant_type.as_str() {
        "authorization_code" => code_exchange(provider, form, resource).await,
        "refresh_token" => refresh_exchange(provider, form, resource).await,
        _ => Err(OAuthProviderError::UnsupportedGrantType),
    }
}

async fn code_exchange(
    provider: &OAuthProvider,
    form: TokenForm,
    resource: String,
) -> Result<OAuthTokenResponse, OAuthProviderError> {
    provider
        .exchange_code(CodeExchangeRequest {
            client_id: required(form.client_id)?,
            client_secret: required(form.client_secret)?,
            code: required(form.code)?,
            redirect_uri: required(form.redirect_uri)?,
            code_verifier: required(form.code_verifier)?,
            resource,
        })
        .await
}

async fn refresh_exchange(
    provider: &OAuthProvider,
    form: TokenForm,
    resource: String,
) -> Result<OAuthTokenResponse, OAuthProviderError> {
    provider
        .exchange_refresh(RefreshExchangeRequest {
            client_id: required(form.client_id)?,
            client_secret: required(form.client_secret)?,
            refresh_token: required(form.refresh_token)?,
            scopes: split_scopes(form.scope.as_deref()),
            resource,
        })
        .await
}

fn required(value: Option<String>) -> Result<String, OAuthProviderError> {
    value.ok_or_else(|| OAuthProviderError::InvalidRequest("Required field is missing.".to_owned()))
}

pub(super) fn split_scopes(value: Option<&str>) -> Vec<String> {
    value
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}
