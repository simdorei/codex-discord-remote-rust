use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::json;
use url::Url;

use crate::oauth::OAuthProviderError;

pub(super) fn oauth_error(error: &OAuthProviderError) -> Response {
    let (status, code, description) = oauth_error_parts(error);
    let mut response = (
        status,
        axum::Json(json!({
            "error": code,
            "error_description": description,
        })),
    )
        .into_response();
    no_store(&mut response);
    response
}

pub(super) fn oauth_error_parts(
    error: &OAuthProviderError,
) -> (StatusCode, &'static str, &'static str) {
    match error {
        OAuthProviderError::InvalidRequest(_) => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "The OAuth request is invalid.",
        ),
        OAuthProviderError::InvalidClient => (
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "OAuth client authentication failed.",
        ),
        OAuthProviderError::InvalidGrant => (
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "The authorization grant is no longer valid.",
        ),
        OAuthProviderError::InvalidScope => (
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "The requested OAuth scope is not allowed.",
        ),
        OAuthProviderError::UnsupportedGrantType => (
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "The OAuth grant type is not supported.",
        ),
        OAuthProviderError::TemporarilyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "OAuth capacity is temporarily unavailable.",
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "The OAuth server could not complete the request.",
        ),
    }
}

pub(super) fn approval_html(content: &str, status: StatusCode) -> Response {
    let page = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Connect local project</title><main>{content}</main></html>"
    );
    let mut response = (status, Html(page)).into_response();
    no_store(&mut response);
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'",
        ),
    );
    response
}

pub(super) fn no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
}

pub(super) fn endpoint_text(base: &Url, suffix: &str) -> String {
    format!("{}{suffix}", base.as_str().trim_end_matches('/'))
}

pub(super) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
