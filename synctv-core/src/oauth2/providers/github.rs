//! GitHub OAuth2 provider

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

/// GitHub OAuth2 provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
}

/// GitHub OAuth2 provider
pub struct GitHubProvider {
    client: Arc<BasicClient>,
    http_client: Arc<Client>,
}

impl GitHubProvider {
    /// Create a new GitHub provider with configuration
    pub fn create(client_id: String, client_secret: String, redirect_url: String) -> Self {
        let client = Arc::new(
            BasicClient::new(
                ClientId::new(client_id),
                Some(ClientSecret::new(client_secret)),
                AuthUrl::new("https://github.com/login/oauth/authorize".to_string()).unwrap(),
                Some(TokenUrl::new("https://github.com/login/oauth/access_token".to_string()).unwrap()),
            )
            .set_redirect_uri(RedirectUrl::new(redirect_url).unwrap()),
        );

        Self {
            client,
            http_client: Arc::new(Client::new()),
        }
    }
}

#[async_trait]
impl Provider for GitHubProvider {
    fn provider_type(&self) -> &str {
        "github"
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
            .map_err(|e| Error::Internal(format!("Failed to exchange code: {}", e)))?;

        // Fetch user info
        let resp = self
            .http_client
            .get("https://api.github.com/user")
            .header("Authorization", format!("Bearer {}", token.access_token().secret()))
            .header("User-Agent", "synctv-rs")
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch user info: {}", e)))?
            .error_for_status()
            .map_err(|e| Error::Internal(format!("GitHub API error: {}", e)))?;

        #[derive(Deserialize)]
        struct GitHubUser {
            login: String,
            id: u64,
            email: Option<String>,
            avatar_url: Option<String>,
        }

        let user: GitHubUser = resp
            .json()
            .await
            .map_err(|e| Error::Internal(format!("Failed to parse user info: {}", e)))?;

        Ok(OAuth2UserInfo {
            provider_user_id: user.id.to_string(),
            username: user.login,
            email: user.email,
            avatar: user.avatar_url,
        })
    }
}

/// Factory function for GitHub provider
pub fn github_factory(config: &serde_yaml::Value) -> Result<Box<dyn Provider>, Error> {
    let config: GitHubConfig = serde_yaml::from_value(config.clone())
        .map_err(|e| Error::InvalidInput(format!("Invalid GitHub config: {}", e)))?;

    Ok(Box::new(GitHubProvider::create(
        config.client_id,
        config.client_secret,
        config.redirect_url,
    )))
}
