use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct AuthorizationQuery {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ApprovalQuery {
    pub request_id: String,
}

#[derive(Deserialize)]
pub(super) struct ApprovalForm {
    pub request_id: String,
    pub owner_token: String,
}

#[derive(Deserialize)]
pub(super) struct TokenForm {
    pub grant_type: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RevokeForm {
    pub token: String,
    pub client_id: String,
    pub client_secret: String,
}
