//! Typed client for the OIS backend REST API.
//!
//! The Discord bot uses this to (a) poll/ack the `integration.outbound_jobs` queue
//! and (b) call back into the API as a service account when a user interacts with a
//! message (e.g. claims an ACE request).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unexpected status: {0}")]
    Status(u16),
}

impl ClientError {
    /// The HTTP status, when this error came from a non-success response.
    pub fn status(&self) -> Option<u16> {
        match self {
            ClientError::Status(code) => Some(*code),
            ClientError::Http(_) => None,
        }
    }
}

/// One outbound job leased from the queue. `payload` shape depends on `job_type`.
#[derive(Debug, Clone, Deserialize)]
pub struct OutboundJob {
    pub id: String,
    pub job_type: String,
    pub payload: Value,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
    pub attempt_count: i32,
}

#[derive(Debug, Serialize)]
struct AckBody<'a> {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// What the bot needs to render the claim time-selectors for a request.
#[derive(Debug, Clone, Deserialize)]
pub struct AceInfo {
    pub event_title: String,
    pub window_label: String,
    pub slots: i32,
    pub claims_count: i64,
    pub time_options: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DiscordAceClaimBody<'a> {
    discord_user_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_hhmm: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_hhmm: Option<&'a str>,
}

/// Handle to the OIS API, authenticated with a service-account bearer token.
#[derive(Clone)]
pub struct OisClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl OisClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Lease up to `limit` pending outbound jobs (marks them in-progress). Needs `integration.jobs.update`.
    pub async fn lease_jobs(&self, limit: u32) -> Result<Vec<OutboundJob>, ClientError> {
        let resp = self
            .http
            .post(self.url("/api/v1/integration/jobs/lease"))
            .bearer_auth(&self.token)
            .query(&[("limit", limit)])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(resp.json::<Vec<OutboundJob>>().await?)
    }

    /// Acknowledge a leased job: success (with an optional result payload) or failure (with an error).
    pub async fn ack_job(
        &self,
        id: &str,
        success: bool,
        result: Option<Value>,
        error: Option<&str>,
    ) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(self.url(&format!("/api/v1/integration/jobs/{id}/ack")))
            .bearer_auth(&self.token)
            .json(&AckBody {
                success,
                result,
                error,
            })
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(())
    }

    /// Claim an ACE request on behalf of a Discord user (resolved to the linked OIS user server-side).
    /// A `403` means that Discord account isn't linked; a `409` means it was already claimed.
    /// The event window + pre-computed HHMM slot options for a request's claim selectors.
    pub async fn ace_info(&self, request_id: &str) -> Result<AceInfo, ClientError> {
        let resp = self
            .http
            .get(self.url(&format!("/api/v1/integration/discord/ace/{request_id}")))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(resp.json::<AceInfo>().await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn claim_ace_via_discord(
        &self,
        request_id: &str,
        discord_user_id: &str,
        notes: Option<&str>,
        start_hhmm: Option<&str>,
        end_hhmm: Option<&str>,
    ) -> Result<Value, ClientError> {
        let resp = self
            .http
            .post(self.url(&format!(
                "/api/v1/integration/discord/ace/{request_id}/claim"
            )))
            .bearer_auth(&self.token)
            .json(&DiscordAceClaimBody {
                discord_user_id,
                notes,
                start_hhmm,
                end_hhmm,
            })
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(resp.json::<Value>().await?)
    }
}
