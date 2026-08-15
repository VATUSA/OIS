//! VATSIM OAuth client, ported from osmium. Resolves dev vs. prod hosts, supports
//! Basic and POST client auth, and parses the userinfo response defensively across
//! the several shapes VATSIM Connect has returned.

use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use crate::{config::vatsim_dev_mode_enabled, errors::ApiError};

#[derive(Clone)]
pub struct VatsimOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub redirect_uri: String,
    pub scope: String,
    pub client_auth_method: ClientAuthMethod,
}

#[derive(Debug, Clone, Copy)]
pub enum ClientAuthMethod {
    Basic,
    Post,
}

#[derive(Debug, Clone)]
pub struct VatsimProfile {
    pub cid: i64,
    pub email: String,
    pub display_name: String,
    pub rating: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

impl VatsimOAuthConfig {
    pub fn from_env() -> Result<Self, ApiError> {
        let use_dev_hosts = vatsim_dev_mode_enabled();
        let client_auth_method = resolve_client_auth_method(use_dev_hosts);

        let authorize_url = resolve_oauth_endpoint(
            "VATSIM_AUTHORIZE_URL",
            "https://auth.vatsim.net/oauth/authorize",
            "https://auth-dev.vatsim.net/oauth/authorize",
            use_dev_hosts,
        );
        let token_url = resolve_oauth_endpoint(
            "VATSIM_TOKEN_URL",
            "https://auth.vatsim.net/oauth/token",
            "https://auth-dev.vatsim.net/oauth/token",
            use_dev_hosts,
        );
        let userinfo_url = resolve_oauth_endpoint(
            "VATSIM_USERINFO_URL",
            "https://auth.vatsim.net/api/user",
            "https://auth-dev.vatsim.net/api/user",
            use_dev_hosts,
        );

        Ok(Self {
            client_id: read_required("VATSIM_CLIENT_ID")?,
            client_secret: read_required("VATSIM_CLIENT_SECRET")?,
            authorize_url: validate_url("VATSIM_AUTHORIZE_URL", &authorize_url)?,
            token_url: validate_url("VATSIM_TOKEN_URL", &token_url)?,
            userinfo_url: validate_url("VATSIM_USERINFO_URL", &userinfo_url)?,
            redirect_uri: validate_url(
                "VATSIM_REDIRECT_URI",
                &read_required("VATSIM_REDIRECT_URI")?,
            )?,
            scope: std::env::var("VATSIM_SCOPE")
                .unwrap_or_else(|_| "full_name email vatsim_details country".to_string()),
            client_auth_method,
        })
    }

    pub fn authorization_url(&self, state: &str) -> Result<String, ApiError> {
        let mut url = Url::parse(&self.authorize_url).map_err(|_| ApiError::Internal)?;
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
    config: &VatsimOAuthConfig,
    code: &str,
) -> Result<String, ApiError> {
    let client = reqwest::Client::new();
    let grant_form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", config.redirect_uri.as_str()),
    ];

    let request = if matches!(config.client_auth_method, ClientAuthMethod::Basic) {
        client
            .post(&config.token_url)
            .form(&grant_form)
            .basic_auth(&config.client_id, Some(&config.client_secret))
    } else {
        let post_form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", config.redirect_uri.as_str()),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
        ];
        client.post(&config.token_url).form(&post_form)
    };

    let response = request
        .send()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;

    if !status.is_success() {
        tracing::error!(
            %status,
            body = body.as_str(),
            auth_method = ?config.client_auth_method,
            "vatsim token exchange failed"
        );
        return Err(if status.as_u16() == 400 || status.as_u16() == 401 {
            ApiError::Unauthorized
        } else {
            ApiError::ServiceUnavailable
        });
    }

    let token = serde_json::from_str::<TokenResponse>(&body).map_err(|_| {
        tracing::error!(
            body = body.as_str(),
            "failed to parse vatsim token response"
        );
        ApiError::Internal
    })?;

    if token.access_token.trim().is_empty() {
        return Err(ApiError::Unauthorized);
    }

    Ok(token.access_token)
}

pub async fn fetch_profile(
    config: &VatsimOAuthConfig,
    access_token: &str,
) -> Result<VatsimProfile, ApiError> {
    let client = reqwest::Client::new();
    let response = client
        .get(&config.userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;

    let status = response.status();
    if !status.is_success() {
        return Err(if status.as_u16() == 401 || status.as_u16() == 403 {
            ApiError::Unauthorized
        } else {
            ApiError::ServiceUnavailable
        });
    }

    let body = response.text().await.map_err(|_| ApiError::Internal)?;
    let userinfo = serde_json::from_str::<Value>(&body).map_err(|error| {
        tracing::error!(
            ?error,
            body = body.as_str(),
            "failed to parse vatsim userinfo"
        );
        ApiError::Internal
    })?;

    parse_profile(userinfo)
}

fn parse_profile(raw: Value) -> Result<VatsimProfile, ApiError> {
    let cid = find_number(
        &raw,
        &[
            "cid",
            "id",
            "data.cid",
            "vatsim_details.cid",
            "data.vatsim_details.cid",
        ],
    )
    .or_else(|| {
        find_string(
            &raw,
            &[
                "cid",
                "id",
                "data.cid",
                "data.user.cid",
                "vatsim_details.cid",
                "data.vatsim_details.cid",
            ],
        )
        .and_then(|v| v.parse().ok())
    })
    .ok_or_else(|| {
        tracing::error!(payload = ?raw, "vatsim profile missing cid");
        ApiError::Internal
    })?;

    let email = find_string(
        &raw,
        &[
            "email",
            "data.email",
            "data.user.email",
            "data.personal.email",
            "personal.email",
        ],
    )
    .ok_or_else(|| {
        tracing::error!(payload = ?raw, "vatsim profile missing email");
        ApiError::Internal
    })?;

    let display_name = find_string(
        &raw,
        &[
            "full_name",
            "name",
            "data.full_name",
            "data.name",
            "data.user.full_name",
            "data.personal.name_full",
            "personal.name_full",
        ],
    )
    .unwrap_or_else(|| format!("CID {cid}"));

    let rating = find_string(
        &raw,
        &[
            "rating",
            "rating_short",
            "vatsim_details.rating",
            "vatsim_details.rating_short",
            "vatsim_details.rating.short",
            "data.vatsim_details.rating",
            "data.vatsim_details.rating_short",
            "data.vatsim_details.rating.short",
        ],
    )
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());

    Ok(VatsimProfile {
        cid,
        email,
        display_name,
        rating,
    })
}

fn read_required(name: &str) -> Result<String, ApiError> {
    let value = std::env::var(name).map_err(|_| {
        tracing::error!(missing_env = name, "missing required oauth env var");
        ApiError::ServiceUnavailable
    })?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        tracing::error!(missing_env = name, "oauth env var cannot be empty");
        return Err(ApiError::ServiceUnavailable);
    }
    Ok(trimmed.to_string())
}

fn resolve_oauth_endpoint(
    env_name: &str,
    prod_default: &str,
    dev_default: &str,
    use_dev_hosts: bool,
) -> String {
    let configured = std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string());

    if let Some(url) = configured {
        if use_dev_hosts && url == prod_default {
            return dev_default.to_string();
        }
        return url;
    }

    if use_dev_hosts {
        dev_default.to_string()
    } else {
        prod_default.to_string()
    }
}

fn resolve_client_auth_method(use_dev_hosts: bool) -> ClientAuthMethod {
    let configured = std::env::var("VATSIM_CLIENT_AUTH_METHOD")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase());

    match configured.as_deref() {
        Some("post") => ClientAuthMethod::Post,
        Some("basic") if use_dev_hosts => {
            tracing::warn!("VATSIM_CLIENT_AUTH_METHOD=basic remapped to post (dev mode)");
            ClientAuthMethod::Post
        }
        Some("basic") => ClientAuthMethod::Basic,
        _ if use_dev_hosts => ClientAuthMethod::Post,
        _ => ClientAuthMethod::Basic,
    }
}

fn validate_url(name: &str, raw: &str) -> Result<String, ApiError> {
    let url = Url::parse(raw).map_err(|_| {
        tracing::error!(env = name, value = raw, "oauth url is not valid");
        ApiError::ServiceUnavailable
    })?;

    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        tracing::error!(
            env = name,
            value = raw,
            "oauth url must have http(s) scheme + host"
        );
        return Err(ApiError::ServiceUnavailable);
    }

    Ok(url.into())
}

fn find_string(value: &Value, paths: &[&str]) -> Option<String> {
    for path in paths {
        if let Some(raw) = get_path(value, path)
            && let Some(s) = raw.as_str()
        {
            return Some(s.to_string());
        }
    }
    None
}

fn find_number(value: &Value, paths: &[&str]) -> Option<i64> {
    for path in paths {
        if let Some(raw) = get_path(value, path)
            && let Some(num) = raw.as_i64()
        {
            return Some(num);
        }
    }
    None
}

fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::parse_profile;
    use serde_json::json;

    #[test]
    fn parses_rating_from_nested_vatsim_details() {
        let profile = parse_profile(json!({
            "cid": 10000010,
            "email": "auth.dev10@vatsim.net",
            "full_name": "Web Ten",
            "data": { "vatsim_details": { "rating": { "short": "S3" } } }
        }))
        .unwrap();

        assert_eq!(profile.cid, 10000010);
        assert_eq!(profile.rating.as_deref(), Some("S3"));
    }
}
