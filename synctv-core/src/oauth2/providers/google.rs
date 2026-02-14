//! Google `OAuth2` provider

use crate::oauth2::{Provider, OAuth2UserInfo};
use crate::Error;
use async_trait::async_trait;
use oauth2::{
    basic::BasicClient,
    AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenUrl, TokenResponse,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Google `OAuth2` provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
}

/// Google `OAuth2` provider
pub struct GoogleProvider {
    client: Arc<BasicClient>,
    http_client: Arc<Client>,
}

impl GoogleProvider {
    /// Create a new Google provider with configuration
    ///
    /// # Errors
    /// Returns error if `redirect_url` is not a valid URL.
    pub fn create(client_id: String, client_secret: String, redirect_url: String) -> Result<Self, Error> {
        let redirect = RedirectUrl::new(redirect_url)
            .map_err(|e| Error::InvalidInput(format!("Invalid Google OAuth2 redirect URL: {e}")))?;
        let client = Arc::new(
            BasicClient::new(
                ClientId::new(client_id),
                Some(ClientSecret::new(client_secret)),
                AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".to_string()).expect("valid Google auth URL"),
                Some(TokenUrl::new("https://oauth2.googleapis.com/token".to_string()).expect("valid Google token URL")),
            )
            .set_redirect_uri(redirect),
        );

        Ok(Self {
            client,
            http_client: Arc::new(Client::new()),
        })
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    fn provider_type(&self) -> &'static str {
        "google"
    }

    async fn new_auth_url(&self, state: &str) -> Result<String, Error> {
        let (auth_url, _csrf_token) = self
            .client
            .authorize_url(|| oauth2::CsrfToken::new(state.to_string()))
            .url();
        Ok(auth_url.to_string())
    }

    async fn get_user_info(&self, code: &str) -> Result<OAuth2UserInfo, Error> {
        // Exchange code for token
        let token = self
            .client
            .exchange_code(oauth2::AuthorizationCode::new(code.to_string()))
            .request_async(oauth2::reqwest::async_http_client)
            .await
            .map_err(|e| Error::Internal(format!("Failed to exchange code: {e}")))?;

        // Fetch user info
        let resp = self
            .http_client
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .header("Authorization", format!("Bearer {}", token.access_token().secret()))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch user info: {e}")))?
            .error_for_status()
            .map_err(|e| Error::Internal(format!("Google API error: {e}")))?;

        #[derive(Deserialize)]
        struct GoogleUser {
            id: String,
            name: String,
            email: String,
            picture: Option<String>,
        }

        let user: GoogleUser = resp
            .json()
            .await
            .map_err(|e| Error::Internal(format!("Failed to parse user info: {e}")))?;

        Ok(OAuth2UserInfo {
            provider_user_id: user.id,
            username: user.name,
            email: Some(user.email),
            avatar: user.picture,
        })
    }
}

/// Factory function for Google provider
pub fn google_factory(config: &serde_json::Value) -> Result<Box<dyn Provider>, Error> {
    let config: GoogleConfig = serde_json::from_value(config.clone())
        .map_err(|e| Error::InvalidInput(format!("Invalid Google config: {e}")))?;

    Ok(Box::new(GoogleProvider::create(
        config.client_id,
        config.client_secret,
        config.redirect_url,
    )?))
}
