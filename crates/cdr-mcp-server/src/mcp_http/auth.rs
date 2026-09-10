use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::oauth::OAuthProvider;
use crate::oauth_store::OAuthTokenRecord;
use crate::scopes::FILES_READ;

#[derive(Clone)]
pub(super) struct AuthorizedAccess(pub OAuthTokenRecord);

pub(super) async fn require_oauth(
    State(provider): State<OAuthProvider>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_bearer);
    let Some(token) = token else {
        return unauthorized(&provider);
    };
    match provider.authenticate_access(token).await {
        Ok(Some(access))
            if access.subject.is_some()
                && access.scopes.iter().any(|scope| scope == FILES_READ) =>
        {
            request.extensions_mut().insert(AuthorizedAccess(access));
            next.run(request).await
        }
        Ok(_) => unauthorized(&provider),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn parse_bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() && !token.contains(' '))
        .then_some(token)
}

fn unauthorized(provider: &OAuthProvider) -> Response {
    let metadata = format!(
        "{}/.well-known/oauth-protected-resource/mcp",
        provider.public_base_url().as_str().trim_end_matches('/')
    );
    let challenge = format!("Bearer resource_metadata=\"{metadata}\", scope=\"files:read\"");
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    let Ok(value) = HeaderValue::from_str(&challenge) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, value);
    response
}
