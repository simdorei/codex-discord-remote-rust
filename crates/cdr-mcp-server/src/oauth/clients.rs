use serde_json::{Value, json};
use url::Url;

use super::util::{new_secret, now, validate_scopes};
use super::{OAuthProvider, OAuthProviderError};
use crate::oauth_store::OAuthStoreError;
use crate::scopes::ALL;

impl OAuthProvider {
    pub async fn register_client(
        &self,
        mut registration: Value,
    ) -> Result<Value, OAuthProviderError> {
        let object = registration.as_object_mut().ok_or_else(|| {
            OAuthProviderError::InvalidRequest("Client registration must be an object.".to_owned())
        })?;
        validate_redirects(object.get("redirect_uris"))?;
        validate_string_choice(
            object.get("token_endpoint_auth_method"),
            "client_secret_post",
            "Unsupported token endpoint authentication method.",
        )?;
        validate_string_array(
            object.get("grant_types"),
            &["authorization_code", "refresh_token"],
            "Unsupported OAuth grant type.",
        )?;
        validate_string_array(
            object.get("response_types"),
            &["code"],
            "Unsupported OAuth response type.",
        )?;
        let scopes = object.get("scope").and_then(Value::as_str).map_or_else(
            || ALL.iter().map(ToString::to_string).collect::<Vec<_>>(),
            |value| value.split_whitespace().map(str::to_owned).collect(),
        );
        validate_scopes(&scopes)?;
        let client_id = new_secret(1);
        let client_secret = new_secret(2);
        object.insert("client_id".to_owned(), json!(client_id));
        object.insert("client_secret".to_owned(), json!(client_secret));
        object.insert("client_id_issued_at".to_owned(), json!(now()?));
        object.insert("client_secret_expires_at".to_owned(), json!(0));
        object.insert(
            "token_endpoint_auth_method".to_owned(),
            json!("client_secret_post"),
        );
        object.insert(
            "grant_types".to_owned(),
            json!(["authorization_code", "refresh_token"]),
        );
        object.insert("response_types".to_owned(), json!(["code"]));
        object.insert("scope".to_owned(), json!(scopes.join(" ")));
        let payload = serde_json::to_string(&registration)?;
        let store = self.store.clone();
        let saved_client_id = client_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            store.save_client_payload(&saved_client_id, &payload)
        })
        .await
        .map_err(|error| OAuthProviderError::StorageTask(error.to_string()))?;
        match result {
            Ok(()) => Ok(registration),
            Err(OAuthStoreError::ClientLimit) => Err(OAuthProviderError::TemporarilyUnavailable),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn is_registered_redirect(
        &self,
        client_id: &str,
        redirect_uri: &str,
    ) -> Result<bool, OAuthProviderError> {
        Ok(self
            .load_client(client_id)
            .await?
            .is_some_and(|client| client.redirect_uris.iter().any(|uri| uri == redirect_uri)))
    }
}

fn validate_redirects(value: Option<&Value>) -> Result<(), OAuthProviderError> {
    let redirects = value.and_then(Value::as_array).ok_or_else(|| {
        OAuthProviderError::InvalidRequest("redirect_uris must be a non-empty array.".to_owned())
    })?;
    if redirects.is_empty() {
        return Err(OAuthProviderError::InvalidRequest(
            "redirect_uris must be a non-empty array.".to_owned(),
        ));
    }
    for redirect in redirects {
        let raw = redirect.as_str().ok_or_else(|| {
            OAuthProviderError::InvalidRequest("redirect_uris must contain URLs.".to_owned())
        })?;
        let parsed = Url::parse(raw).map_err(|_| {
            OAuthProviderError::InvalidRequest("redirect_uris contains an invalid URL.".to_owned())
        })?;
        if parsed.fragment().is_some() || !matches!(parsed.scheme(), "http" | "https") {
            return Err(OAuthProviderError::InvalidRequest(
                "redirect_uris must use HTTP(S) and omit fragments.".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_string_choice(
    value: Option<&Value>,
    allowed: &str,
    message: &str,
) -> Result<(), OAuthProviderError> {
    if value.is_none_or(|value| value.as_str() == Some(allowed)) {
        Ok(())
    } else {
        Err(OAuthProviderError::InvalidRequest(message.to_owned()))
    }
}

fn validate_string_array(
    value: Option<&Value>,
    allowed: &[&str],
    message: &str,
) -> Result<(), OAuthProviderError> {
    let Some(values) = value else {
        return Ok(());
    };
    let values = values
        .as_array()
        .filter(|values| !values.is_empty())
        .ok_or_else(|| OAuthProviderError::InvalidRequest(message.to_owned()))?;
    if values
        .iter()
        .all(|item| item.as_str().is_some_and(|item| allowed.contains(&item)))
    {
        Ok(())
    } else {
        Err(OAuthProviderError::InvalidRequest(message.to_owned()))
    }
}
