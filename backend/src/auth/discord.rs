//! Discord OAuth client for **account linking** (identify scope only). Unlike the VATSIM flow this
//! never creates a session — it proves ownership of a Discord account and records the mapping so the
//! bot can act on behalf of the OIS user. Config is optional: absent env just disables linking.

use reqwest::Url;
use serde::Deserialize;

use crate::errors::ApiError;

const AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";
const TOKEN_URL: &str = "https://discord.com/api/oauth2/token";
const USER_URL: &str = "https://discord.com/api/users/@me";

#[derive(Clone)]
pub struct DiscordOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scope: String,
}

/// The Discord identity behind an access token.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscordIdentity {
    pub id: String,
    pub username: String,
    /// The user's display name (nullable in Discord's API).
    pub global_name: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

impl DiscordOAuthConfig {
    /// Build from env, or `None` when linking isn't configured (so the feature degrades cleanly
    /// instead of erroring at startup).
    pub fn from_env() -> Option<Self> {
        let client_id = trimmed("DISCORD_CLIENT_ID")?;
        let client_secret = trimmed("DISCORD_CLIENT_SECRET")?;
        let redirect_uri = trimmed("DISCORD_REDIRECT_URI")?;
        Some(Self {
            client_id,
            client_secret,
            redirect_uri,
            scope: std::env::var("DISCORD_SCOPE").unwrap_or_else(|_| "identify".to_string()),
        })
    }

    pub fn authorization_url(&self, state: &str) -> Result<String, ApiError> {
        let mut url = Url::parse(AUTHORIZE_URL).map_err(|_| ApiError::Internal)?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", &self.scope)
            .append_pair("state", state);
        Ok(url.into())
    }
}

pub async fn exchange_code_for_token(
    config: &DiscordOAuthConfig,
    code: &str,
) -> Result<String, ApiError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
    ];
    let response = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;
    if !status.is_success() {
        tracing::error!(%status, body = body.as_str(), "discord token exchange failed");
        return Err(if matches!(status.as_u16(), 400 | 401) {
            ApiError::Unauthorized
        } else {
            ApiError::ServiceUnavailable
        });
    }

    let token = serde_json::from_str::<TokenResponse>(&body).map_err(|_| ApiError::Internal)?;
    if token.access_token.trim().is_empty() {
        return Err(ApiError::Unauthorized);
    }
    Ok(token.access_token)
}

pub async fn fetch_identity(access_token: &str) -> Result<DiscordIdentity, ApiError> {
    let response = reqwest::Client::new()
        .get(USER_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;
    if !response.status().is_success() {
        return Err(ApiError::Unauthorized);
    }
    response
        .json::<DiscordIdentity>()
        .await
        .map_err(|_| ApiError::Internal)
}

fn trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
