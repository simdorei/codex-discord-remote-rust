use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::forms::AuthorizationQuery;
use super::responses::{endpoint_text, escape_html, oauth_error, oauth_error_parts};
use crate::oauth::{OAuthProvider, OAuthProviderError};

pub(super) fn redirect(location: &str) -> Response {
    let Ok(location) = HeaderValue::from_str(location) else {
        return oauth_error(&OAuthProviderError::Configuration(
            "OAuth redirect URL could not be encoded.",
        ));
    };
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

pub(super) fn authorization_error_redirect(
    provider: &OAuthProvider,
    query: &AuthorizationQuery,
    error: &OAuthProviderError,
) -> Response {
    let (_, code, description) = oauth_error_parts(error);
    let Ok(mut redirect_uri) = query.redirect_uri.parse::<url::Url>() else {
        return oauth_error(&OAuthProviderError::InvalidRequest(
            "redirect_uri is invalid.".to_owned(),
        ));
    };
    let issuer = provider.public_base_url().as_str().trim_end_matches('/');
    let mut pairs = redirect_uri.query_pairs_mut();
    pairs
        .append_pair("error", code)
        .append_pair("error_description", description)
        .append_pair("iss", issuer);
    if let Some(state) = &query.state {
        pairs.append_pair("state", state);
    }
    drop(pairs);
    redirect(redirect_uri.as_str())
}

pub(super) fn approval_form(
    provider: &OAuthProvider,
    request_id: &str,
    scopes: &[String],
    message: Option<&str>,
) -> String {
    let feedback = message
        .map(|message| format!("<p role=\"alert\">{}</p>", escape_html(message)))
        .unwrap_or_default();
    let permissions = scopes.iter().fold(String::new(), |mut output, scope| {
        write!(&mut output, "<li><code>{}</code></li>", escape_html(scope))
            .expect("writing HTML into a String cannot fail");
        output
    });
    format!(
        "<h1>Connect ChatGPT to your local project</h1>\
         <p>Enter the private owner token stored for this MCP server.</p>{feedback}\
         <ul>{permissions}</ul><form method=\"post\" action=\"{}\">\
         <input type=\"hidden\" name=\"request_id\" value=\"{}\">\
         <label>Owner token<input type=\"password\" name=\"owner_token\" required></label>\
         <button type=\"submit\">Connect</button></form>",
        escape_html(&endpoint_text(provider.public_base_url(), "/oauth/approve")),
        escape_html(request_id),
    )
}
use std::fmt::Write as _;
