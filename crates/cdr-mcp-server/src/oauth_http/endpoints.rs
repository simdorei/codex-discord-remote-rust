use axum::Json;
use axum::extract::{Form, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use super::forms::{ApprovalForm, ApprovalQuery, AuthorizationQuery, RevokeForm, TokenForm};
use super::responses::{approval_html, no_store, oauth_error};
use super::token_endpoint::{exchange, split_scopes};
use super::views::{approval_form, authorization_error_redirect, redirect};
use crate::oauth::{AuthorizationRequest, OAuthProvider, OAuthProviderError};

pub(super) async fn register(
    State(provider): State<OAuthProvider>,
    Json(registration): Json<Value>,
) -> Response {
    match provider.register_client(registration).await {
        Ok(client) => {
            let mut response = (StatusCode::CREATED, Json(client)).into_response();
            no_store(&mut response);
            response
        }
        Err(error) => oauth_error(&error),
    }
}

pub(super) async fn authorize(
    State(provider): State<OAuthProvider>,
    Query(query): Query<AuthorizationQuery>,
) -> Response {
    let request = AuthorizationRequest {
        client_id: query.client_id.clone(),
        redirect_uri: query.redirect_uri.clone(),
        state: query.state.clone(),
        code_challenge: query.code_challenge.clone(),
        code_challenge_method: query.code_challenge_method.clone(),
        scopes: split_scopes(query.scope.as_deref()),
        resource: query
            .resource
            .clone()
            .unwrap_or_else(|| provider.resource_url().to_string()),
    };
    let result = if query.response_type == "code" {
        provider.begin_authorization(request).await
    } else {
        Err(OAuthProviderError::InvalidRequest(
            "Only the code response type is supported.".to_owned(),
        ))
    };
    match result {
        Ok(pending) => redirect(pending.approval_url.as_str()),
        Err(error) => {
            match provider
                .is_registered_redirect(&query.client_id, &query.redirect_uri)
                .await
            {
                Ok(true) => authorization_error_redirect(&provider, &query, &error),
                Ok(false) => oauth_error(&error),
                Err(lookup_error) => oauth_error(&lookup_error),
            }
        }
    }
}

pub(super) async fn approval_page(
    State(provider): State<OAuthProvider>,
    Query(query): Query<ApprovalQuery>,
) -> Response {
    match provider.pending_scopes(&query.request_id).await {
        Ok(Some(scopes)) => approval_html(
            &approval_form(&provider, &query.request_id, &scopes, None),
            StatusCode::OK,
        ),
        Ok(None) => approval_html(
            "<h1>This authorization request expired.</h1>",
            StatusCode::BAD_REQUEST,
        ),
        Err(error) => oauth_error(&error),
    }
}

pub(super) async fn approve(
    State(provider): State<OAuthProvider>,
    Form(form): Form<ApprovalForm>,
) -> Response {
    match provider.approve(&form.request_id, &form.owner_token).await {
        Ok(callback) => redirect(callback.as_str()),
        Err(OAuthProviderError::ApprovalDenied) => {
            match provider.pending_scopes(&form.request_id).await {
                Ok(Some(scopes)) => approval_html(
                    &approval_form(
                        &provider,
                        &form.request_id,
                        &scopes,
                        Some("The owner token did not match."),
                    ),
                    StatusCode::UNAUTHORIZED,
                ),
                Ok(None) => approval_html(
                    "<h1>This authorization request expired.</h1>",
                    StatusCode::BAD_REQUEST,
                ),
                Err(error) => oauth_error(&error),
            }
        }
        Err(OAuthProviderError::ApprovalNotFound) => approval_html(
            "<h1>This authorization request expired.</h1>",
            StatusCode::BAD_REQUEST,
        ),
        Err(OAuthProviderError::TemporarilyUnavailable) => approval_html(
            "<h1>OAuth capacity is temporarily unavailable.</h1>",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        Err(error) => oauth_error(&error),
    }
}

pub(super) async fn token(
    State(provider): State<OAuthProvider>,
    Form(form): Form<TokenForm>,
) -> Response {
    match exchange(&provider, form).await {
        Ok(token) => {
            let mut response = Json(token).into_response();
            no_store(&mut response);
            response
        }
        Err(error) => oauth_error(&error),
    }
}

pub(super) async fn revoke(
    State(provider): State<OAuthProvider>,
    Form(form): Form<RevokeForm>,
) -> Response {
    match provider
        .revoke(&form.client_id, &form.client_secret, form.token)
        .await
    {
        Ok(()) => {
            let mut response = StatusCode::OK.into_response();
            no_store(&mut response);
            response
        }
        Err(error) => oauth_error(&error),
    }
}
