//! Typed client for the OIS backend REST API.
//!
//! The Discord bot uses this to (a) poll/ack the `integration.outbound_jobs` queue
//! and (b) call back into the API as a service account when a user interacts with a
//! message (e.g. claims an ACE request). Fleshed out in Phase 4.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unexpected status: {0}")]
    Status(u16),
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
            base_url: base_url.into(),
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
}
