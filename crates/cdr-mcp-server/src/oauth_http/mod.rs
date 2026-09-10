mod endpoints;
mod forms;
mod metadata;
mod responses;
mod token_endpoint;
mod views;

use axum::Router;
use axum::routing::{get, post};

use crate::oauth::OAuthProvider;

pub fn oauth_router(provider: OAuthProvider) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(metadata::protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(metadata::authorization_server),
        )
        .route("/register", post(endpoints::register))
        .route("/authorize", get(endpoints::authorize))
        .route(
            "/oauth/approve",
            get(endpoints::approval_page).post(endpoints::approve),
        )
        .route("/token", post(endpoints::token))
        .route("/revoke", post(endpoints::revoke))
        .with_state(provider)
}
