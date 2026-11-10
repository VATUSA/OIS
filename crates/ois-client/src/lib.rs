//! Typed client for the OIS backend REST API.
//!
//! The Discord bot uses this to (a) poll/ack the `integration.outbound_jobs` queue
//! and (b) call back into the API as a service account when a user interacts with a
//! message (e.g. claims an ACE request).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// How long the bot waits for a TCP connection to the backend before the call fails.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The whole-call deadline: connect, send, and read the response body. Without it one backend
/// connection that stops answering parks `job_loop` on a lease or an ack until the bot restarts
/// (#930).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

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
    /// The `attempt_count` this worker was leased under, so the backend can refuse an ack from a
    /// lease that has already been superseded (VATUSA/OIS#472). Omitted when unknown, which the
    /// backend treats as the old status-only fence.
    #[serde(skip_serializing_if = "Option::is_none")]
    attempt: Option<i32>,
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

/// What the bot needs to reply to a "View structured" button click on a TMI post.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscordTmiInfo {
    pub restriction: String,
    pub decoded: Option<String>,
}

/// What the bot needs to reply to a "View structured" button click on an advisory post
/// (VATUSA/OIS#459).
#[derive(Debug, Clone, Deserialize)]
pub struct DiscordAdvisoryInfo {
    pub kind: String,
    /// Null when the advisory was typed as raw text — the bot reports that rather than showing an
    /// empty breakdown.
    pub structured: Option<serde_json::Value>,
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

#[derive(Debug, Serialize)]
struct AvailabilityBody<'a> {
    discord_user_id: &'a str,
    status: &'a str,
}

/// One channel in a guild snapshot the bot pushes to the backend.
#[derive(Debug, Serialize)]
pub struct GuildChannelSnap {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub parent_id: Option<String>,
    pub position: i32,
}

/// One role in a guild snapshot.
#[derive(Debug, Serialize)]
pub struct GuildRoleSnap {
    pub id: String,
    pub name: String,
    pub managed: bool,
    pub position: i32,
}

/// A snapshot of one guild the bot is in (its real channels + roles), for the config dropdowns.
#[derive(Debug, Serialize)]
pub struct GuildSnap {
    pub guild_id: String,
    pub name: String,
    pub channels: Vec<GuildChannelSnap>,
    pub roles: Vec<GuildRoleSnap>,
}

#[derive(Debug, Serialize)]
struct GuildSnapshotBody {
    guilds: Vec<GuildSnap>,
}

/// Outcome of an availability button press. `ok=false` is a soft refusal — `reason` is
/// `unlinked` | `forbidden` | `invalid` — so the bot can tell the user why.
#[derive(Debug, Clone, Deserialize)]
pub struct AvailabilityResult {
    pub ok: bool,
    pub reason: Option<String>,
    pub display_name: Option<String>,
    pub status: Option<String>,
}

/// Handle to the OIS API, authenticated with a service-account bearer token.
#[derive(Clone)]
pub struct OisClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl OisClient {
    /// Fails only if reqwest cannot build its client (its TLS backend did not initialize).
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            http,
        })
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

    /// Lease up to `limit` of `consumer`'s pending outbound jobs (marks them in-progress). Needs
    /// `integration.jobs.update`. The backend requires `consumer` and returns only that consumer's jobs.
    pub async fn lease_jobs(
        &self,
        consumer: &str,
        limit: u32,
    ) -> Result<Vec<OutboundJob>, ClientError> {
        let resp = self
            .http
            .post(self.url("/api/v1/integration/jobs/lease"))
            .bearer_auth(&self.token)
            .query(&lease_query(consumer, limit))
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
        consumer: &str,
        id: &str,
        success: bool,
        result: Option<Value>,
        error: Option<&str>,
        attempt: Option<i32>,
    ) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(self.url(&format!("/api/v1/integration/jobs/{id}/ack")))
            .bearer_auth(&self.token)
            .query(&ack_query(consumer))
            .json(&AckBody {
                success,
                result,
                error,
                attempt,
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

    /// The raw line + structured breakdown (if any) for a TMI's "View structured" reply.
    pub async fn tmi_info(&self, tmi_id: &str) -> Result<DiscordTmiInfo, ClientError> {
        let resp = self
            .http
            .get(self.url(&format!("/api/v1/integration/discord/tmi/{tmi_id}")))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(resp.json::<DiscordTmiInfo>().await?)
    }

    /// The document type + structured fields (if any) for an advisory's "View structured" reply.
    pub async fn advisory_info(
        &self,
        advisory_id: &str,
    ) -> Result<DiscordAdvisoryInfo, ClientError> {
        let resp = self
            .http
            .get(self.url(&format!(
                "/api/v1/integration/discord/advisory/{advisory_id}"
            )))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(resp.json::<DiscordAdvisoryInfo>().await?)
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

    /// Relay an availability button press (🟢/🟡/🔴) from a DCC event thread. The backend resolves the
    /// Discord user, checks they may respond, and records it — returning a soft `ok`/`reason` result.
    pub async fn set_availability(
        &self,
        event_id: &str,
        discord_user_id: &str,
        status: &str,
    ) -> Result<AvailabilityResult, ClientError> {
        let resp = self
            .http
            .post(self.url(&format!(
                "/api/v1/integration/discord/availability/{event_id}"
            )))
            .bearer_auth(&self.token)
            .json(&AvailabilityBody {
                discord_user_id,
                status,
            })
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(resp.json::<AvailabilityResult>().await?)
    }

    /// Push the guilds the bot is in (channels + roles) so the admin config offers dropdowns. Full
    /// replace of the backend's guild snapshot. Needs `integration.jobs.update` (the BOT role).
    pub async fn push_guild_snapshot(&self, guilds: Vec<GuildSnap>) -> Result<(), ClientError> {
        let resp = self
            .http
            .post(self.url("/api/v1/integration/discord/guilds/snapshot"))
            .bearer_auth(&self.token)
            .json(&GuildSnapshotBody { guilds })
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        Ok(())
    }
}

/// The lease's query string. A function so its wire names can be pinned (#590): the backend 400s a
/// lease without `consumer`, so a rename here stops the bot leasing anything.
fn lease_query(consumer: &str, limit: u32) -> [(&'static str, String); 2] {
    [
        ("consumer", consumer.to_string()),
        ("limit", limit.to_string()),
    ]
}

/// The ack's query string: the backend applies an ack only to the named consumer's job (#590).
fn ack_query(consumer: &str) -> [(&'static str, &str); 1] {
    [("consumer", consumer)]
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{AckBody, ClientError, OisClient, ack_query, lease_query};

    /// AC 2 and AC 3 (#930): a backend that accepts the connection and never answers. The bounds are
    /// D72's values written as literals, so a changed constant fails here too.
    ///
    /// The outer `tokio::time::timeout` is what turns a missing request timeout into a failed test
    /// rather than a stalled one. The clock is real: a paused clock auto-advances past the timers
    /// while the local connect is still in flight.
    #[tokio::test]
    async fn a_lease_against_a_server_that_never_replies_errors_within_the_request_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Accept and hold every connection without writing a byte.
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        let api = OisClient::new(format!("http://{addr}"), "t").unwrap();

        let started = Instant::now();
        let outcome =
            tokio::time::timeout(Duration::from_secs(16), api.lease_jobs("discord", 10)).await;
        let elapsed = started.elapsed();

        let result = outcome.expect("lease_jobs hung past 15 s + 1 s: no request timeout");
        match result {
            Err(ClientError::Http(e)) => assert!(e.is_timeout(), "not a timeout: {e}"),
            other => panic!("expected a timeout error, got {other:?}"),
        }
        assert!(
            elapsed >= Duration::from_secs(15),
            "failed after {elapsed:?}, before the 15 s request timeout"
        );
    }

    /// AC 5 (#930): a connect that never completes. A listener with a backlog of 0 queues one
    /// connection; with that slot taken and nobody accepting, Linux drops the next SYN, so the
    /// client's connect hangs. Linux-only for that reason. The 15 s request timeout would also end
    /// it, so the 6 s bound is what proves the 5 s connect timeout fired.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_connect_that_never_completes_errors_within_the_connect_timeout() {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = socket.listen(0).unwrap();
        let addr = listener.local_addr().unwrap();
        let _fills_the_queue = std::net::TcpStream::connect(addr).unwrap();
        let api = OisClient::new(format!("http://{addr}"), "t").unwrap();

        let started = Instant::now();
        let outcome =
            tokio::time::timeout(Duration::from_secs(6), api.lease_jobs("discord", 10)).await;
        let elapsed = started.elapsed();

        let result = outcome.expect("connect hung past 5 s + 1 s: no connect timeout");
        match result {
            Err(ClientError::Http(e)) => assert!(
                e.is_connect() && e.is_timeout(),
                "not a connect timeout: {e:?}"
            ),
            other => panic!("expected a connect timeout, got {other:?}"),
        }
        assert!(
            elapsed >= Duration::from_secs(5),
            "failed after {elapsed:?}, before the 5 s connect timeout"
        );
        drop(listener);
    }

    /// Pinned against `AckQuery` in `backend/src/handlers/integration.rs`, which requires `consumer`.
    #[test]
    fn an_ack_names_its_consumer_on_the_wire() {
        assert_eq!(ack_query("discord"), [("consumer", "discord")]);
    }

    /// Pinned against `LeaseQuery` in `backend/src/handlers/integration.rs`, which requires `consumer`.
    #[test]
    fn a_lease_names_its_consumer_on_the_wire() {
        assert_eq!(
            lease_query("discord", 10),
            [
                ("consumer", "discord".to_string()),
                ("limit", "10".to_string())
            ]
        );
    }

    /// `AckBody` is hand-written, not generated, and the backend reads the lease token through
    /// `#[serde(default)]` so that an old bot keeps working across a deploy (VATUSA/OIS#472). Together
    /// that means a disagreement about this field's name is silent: the backend deserialises `None`,
    /// falls back to the status-only fence the issue exists to replace, and nothing fails. Nor would
    /// `client-drift` notice — it compares the OpenAPI document to the TypeScript client, and this
    /// crate is neither.
    ///
    /// So the name is asserted against `AckJobRequest`'s field in `backend/src/models/mod.rs`.
    #[test]
    fn the_lease_token_goes_on_the_wire_as_attempt() {
        let body = serde_json::to_value(AckBody {
            success: true,
            result: None,
            error: None,
            attempt: Some(2),
        })
        .unwrap();

        assert_eq!(body.get("attempt").and_then(|v| v.as_i64()), Some(2));
    }

    /// Omitted rather than sent as `null` when there is no lease token, which is what lets an older
    /// backend — one that has never heard of the field — accept the ack unchanged.
    #[test]
    fn no_lease_token_means_the_field_is_absent_not_null() {
        let body = serde_json::to_value(AckBody {
            success: true,
            result: None,
            error: None,
            attempt: None,
        })
        .unwrap();

        assert_eq!(
            body.as_object().map(|o| o.contains_key("attempt")),
            Some(false)
        );
        assert_eq!(body.get("success").and_then(|v| v.as_bool()), Some(true));
    }
}
