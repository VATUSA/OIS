//! VATUSA member sync.
//!
//! - **Daily, the whole division** over v3 (`GET /v3/division/controllers`, #605): every controller is
//!   seeded or refreshed with their roles and visits, and the access mapped from those roles (#548) is
//!   reconciled. Also run on demand — from Background Tasks, or when a verified webhook delivery says
//!   the roster changed.
//! - **At sign-in, one person** over v2 — the one remaining v2 call, kept deliberately: v3 carries no
//!   `discord_id` (which the bot needs for DMs), and it makes a brand-new member's first sign-in
//!   correct before the next daily pull.
//!
//! Everything here no-ops unless `VATUSA_API_KEY` is configured; the webhook additionally needs
//! `OIS_PUBLIC_URL` (the receiver must be public HTTPS) and `OIS_SECRET_KEY` (its secret is encrypted).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};
use sqlx::PgPool;

use crate::config::{ois_public_url, ois_secret_key, vatusa_api_base, vatusa_api_key};
use crate::job_registry::{JobRegistry, run_interval};
use crate::repos::vatusa as repo;

/// How long sign-in waits for VATUSA before continuing without it (#548).
const LOGIN_SYNC_BUDGET: Duration = Duration::from_secs(5);

// --- VATUSA v2 /user/{cid} response ---

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VatusaMember {
    pub cid: i64,
    #[serde(default)]
    pub fname: String,
    #[serde(default)]
    pub lname: String,
    #[serde(default)]
    pub facility: String,
    #[serde(default)]
    pub rating: i32,
    #[serde(default)]
    pub rating_short: Option<String>,
    #[serde(default, deserialize_with = "de_flag")]
    pub flag_homecontroller: bool,
    #[serde(default)]
    pub facility_join: Option<String>,
    /// The member's linked Discord user id (snowflake), as held by VATUSA. VATUSA is the authoritative
    /// source of the OIS↔Discord mapping — OIS never runs its own link flow.
    #[serde(default)]
    pub discord_id: Option<String>,
    #[serde(default)]
    pub roles: Vec<VatusaRole>,
    #[serde(default)]
    pub visiting_facilities: Vec<VatusaVisit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VatusaRole {
    #[serde(default)]
    pub facility: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VatusaVisit {
    #[serde(default)]
    pub facility: String,
}

impl VatusaMember {
    /// Full name from the VATUSA first/last, trimmed.
    pub fn full_name(&self) -> String {
        format!("{} {}", self.fname.trim(), self.lname.trim())
            .trim()
            .to_string()
    }

    /// Short ATC rating code — the API's `rating_short` when present, else mapped from the int.
    pub fn short_rating(&self) -> Option<String> {
        if let Some(s) = self.rating_short.as_ref().filter(|s| !s.is_empty()) {
            return Some(s.clone());
        }
        rating_to_short(self.rating).map(str::to_string)
    }

    pub fn facility_join_ts(&self) -> Option<DateTime<Utc>> {
        self.facility_join
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
    }
}

/// VATUSA numeric rating → short code (used only when the API omits `rating_short`).
fn rating_to_short(rating: i32) -> Option<&'static str> {
    Some(match rating {
        -1 => "INA",
        0 => "SUS",
        1 => "OBS",
        2 => "S1",
        3 => "S2",
        4 => "S3",
        5 => "C1",
        6 => "C2",
        7 => "C3",
        8 => "I1",
        9 => "I2",
        10 => "I3",
        11 => "SUP",
        12 => "ADM",
        _ => return None,
    })
}

/// VATUSA occasionally reports boolean flags as `0`/`1` or strings; accept any of them.
fn de_flag<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().is_some_and(|x| x != 0),
        serde_json::Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    })
}

// --- HTTP ---

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(15))
        .build()
        .expect("failed to build VATUSA HTTP client")
}

/// Fetch one member's details from the VATUSA v2 API — **the one remaining v2 call** (#605), used only
/// at sign-in, for `discord_id` (which v3 does not carry) and a brand-new member's first sign-in.
/// `api_key` is passed as `?apikey=` so visits are populated. (Email is not parsed: sign-in has it from
/// VATSIM Connect.)
///
/// Every error is stripped of its URL: reqwest's `Display` appends the request URL, query and all, so a
/// failed fetch would otherwise write the key into the sign-in warning log (#757).
async fn fetch_member(
    http: &reqwest::Client,
    base: &str,
    api_key: &str,
    cid: i64,
) -> Result<VatusaMember, reqwest::Error> {
    let url = format!("{base}/v2/user/{cid}");
    let env: Envelope<VatusaMember> = async {
        http.get(&url)
            .query(&[("apikey", api_key)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    .await
    .map_err(reqwest::Error::without_url)?;
    Ok(env.data)
}

/// Fetch a member and persist their details/roles/visits — sign-in's sync. Best-effort: errors are
/// returned for the caller to log, never to fail the surrounding flow.
pub async fn sync_member(pool: &PgPool, cid: i64) -> Result<(), String> {
    let Some(api_key) = vatusa_api_key() else {
        return Ok(()); // sync disabled
    };
    let member = fetch_member(&client(), &vatusa_api_base(), &api_key, cid)
        .await
        .map_err(|e| format!("VATUSA fetch for {cid} failed: {e}"))?;
    repo::upsert_member(pool, &member)
        .await
        .map_err(|e| format!("VATUSA upsert for {cid} failed: {e}"))
}

/// Sync a member as part of sign-in, **awaited**, so a first-ever login already holds the access its
/// VATUSA roles map to when the session is issued (#548). Before, the sync was detached and a new
/// user's first session saw an empty role table.
///
/// Bounded by `LOGIN_SYNC_BUDGET`: if VATUSA is slow, sign-in proceeds and the sync finishes in the
/// background. A VATUSA error is logged and sign-in proceeds — login never fails because of VATUSA.
pub async fn sync_member_on_login(pool: &PgPool, cid: i64) {
    if vatusa_api_key().is_none() {
        return;
    }
    if !completes_within(LOGIN_SYNC_BUDGET, sync_member(pool, cid)).await {
        spawn_member_sync(pool.clone(), cid);
    }
}

/// Whether `sync` finished — successfully or not — inside `budget`. Split from
/// `sync_member_on_login` so the bound is testable without calling VATUSA.
async fn completes_within(
    budget: Duration,
    sync: impl Future<Output = Result<(), String>>,
) -> bool {
    match tokio::time::timeout(budget, sync).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::warn!("{e}");
            true
        }
        Err(_) => {
            tracing::warn!("VATUSA sync exceeded the sign-in budget; finishing in the background");
            false
        }
    }
}

/// Fire-and-forget a member sync — sign-in's fallback when VATUSA is slower than its budget.
pub fn spawn_member_sync(pool: PgPool, cid: i64) {
    if vatusa_api_key().is_none() {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = sync_member(&pool, cid).await {
            tracing::warn!("{e}");
        }
    });
}

// --- v3 division pull (#605) ---

/// `GET /v3/division/controllers` — the whole division and every role grant, in one response with no
/// envelope and no pagination. Fields OIS doesn't store are not modelled.
#[derive(Debug, Deserialize)]
pub struct ControllersAndRoles {
    pub controllers: Vec<DivisionController>,
    /// Flat across the division — not nested per controller.
    pub roles: Vec<UserRole>,
}

#[derive(Debug, Deserialize)]
pub struct DivisionController {
    pub cid: i64,
    pub display_name: String,
    pub controller_rating: i32,
    #[serde(default)]
    pub facility: String,
    #[serde(default)]
    pub visiting_facilities: Vec<String>,
    /// The nearest v3 has to v2's `facility_join`: when they moved to their current facility.
    #[serde(default)]
    pub last_transfer_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UserRole {
    pub cid: i64,
    /// A facility code, or `*` for a division-wide grant (stored as `ZHQ`).
    pub facility: String,
    pub role: String,
    /// Unix seconds.
    pub granted_at: i64,
}

/// The pull's name on the admin Background Tasks page.
pub const PULL_JOB: &str = "vatusa_division_pull";
const PULL_INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// One transaction per this many controllers: a single transaction over the whole division bloats the
/// WAL, one per controller is thousands of commits.
const PULL_CHUNK: usize = 500;

/// The response, normalised and sorted by CID (the order `apply_division_chunk` must lock rows in).
/// Roles for a CID that isn't in `controllers` are dropped — there is no one to attach them to.
pub fn division_members(pulled: ControllersAndRoles) -> Vec<repo::DivisionMember> {
    let mut roles: std::collections::HashMap<i64, Vec<_>> = std::collections::HashMap::new();
    for r in pulled.roles {
        let (facility, role) = (
            repo::normalise_facility(&r.facility),
            r.role.trim().to_uppercase(),
        );
        if facility.is_empty() || role.is_empty() {
            continue;
        }
        let at = DateTime::from_timestamp(r.granted_at, 0);
        roles.entry(r.cid).or_default().push((facility, role, at));
    }
    let mut members: Vec<_> = pulled
        .controllers
        .into_iter()
        .map(|c| repo::DivisionMember {
            cid: c.cid,
            display_name: c.display_name.trim().to_string(),
            rating_numeric: c.controller_rating,
            rating_short: rating_to_short(c.controller_rating).map(str::to_string),
            facility: repo::normalise_facility(&c.facility),
            facility_join: c.last_transfer_time,
            visits: c
                .visiting_facilities
                .iter()
                .map(|f| repo::normalise_facility(f))
                .filter(|f| !f.is_empty())
                .collect(),
            roles: roles.remove(&c.cid).unwrap_or_default(),
        })
        .collect();
    members.sort_by_key(|m| m.cid);
    members.dedup_by_key(|m| m.cid);
    members
}

/// Store a pulled division: chunked bulk upserts, then clear whoever has left.
///
/// **Refuses a response that looks truncated** — empty, under half the members already synced, or
/// under half the role grants already stored — without writing anything. Absence from the pull
/// removes a controller's VATUSA roles and the access mapped from them, so a partial response applied
/// blindly would be a mass revocation. `roles` is a separate array from `controllers`, so a full
/// roster with a lost or cut-off role list must be caught on its own.
///
/// Returns the summary and whether anyone's roles — and so their mapped access — moved. It holds no
/// realtime sender on purpose: announcing the change is [`apply_and_announce`]'s job, after this has
/// returned, so a nudge can never reach a browser before the data it announces has committed (#644).
pub async fn apply_division(
    pool: &PgPool,
    members: &[repo::DivisionMember],
) -> Result<(String, bool), String> {
    let known = repo::count_synced_members(pool)
        .await
        .map_err(|e| e.to_string())?;
    if members.is_empty() || (members.len() as i64) < known / 2 {
        return Err(format!(
            "refused a division pull of {} controllers against {known} already synced — \
             it looks truncated, and applying it would strip roles from everyone missing",
            members.len()
        ));
    }
    let stored_roles = repo::count_stored_roles(pool)
        .await
        .map_err(|e| e.to_string())?;
    let pulled_roles: usize = members.iter().map(|m| m.roles.len()).sum();
    if (pulled_roles as i64) < stored_roles / 2 {
        return Err(format!(
            "refused a division pull of {pulled_roles} role grants against {stored_roles} stored — \
             its role list looks truncated, and applying it would revoke the access they map to"
        ));
    }

    let (mut seeded, mut changed) = (0, 0);
    for chunk in members.chunks(PULL_CHUNK) {
        let (s, c) = repo::apply_division_chunk(pool, chunk)
            .await
            .map_err(|e| format!("store division chunk: {e}"))?;
        seeded += s;
        changed += c;
    }
    let present: Vec<i64> = members.iter().map(|m| m.cid).collect();
    let departed = repo::clear_departed(pool, &present)
        .await
        .map_err(|e| format!("clear departed members: {e}"))?;
    let summary = format!(
        "{} controllers ({seeded} new); access changed for {changed}; {departed} departed",
        members.len()
    );
    Ok((summary, changed + departed > 0))
}

/// Apply a pull, then tell signed-in browsers if anyone's access moved (#644). The nudge is sent only
/// once [`apply_division`] has returned — every chunk committed and the departed cleared — so a browser
/// refetching `/me` on it reads the new access. A pull that changed nothing, or one refused as
/// truncated, tells no one.
pub async fn apply_and_announce(
    pool: &PgPool,
    members: &[repo::DivisionMember],
    events: &crate::realtime::Events,
) -> Result<String, String> {
    let (summary, access_moved) = apply_division(pool, members).await?;
    if access_moved {
        events.publish(crate::realtime::topic::ACCESS_GRANTED);
    }
    Ok(summary)
}

/// A client for the division pull: a multi-megabyte body, so a much longer timeout than the per-user
/// client's 15 s — which covers the whole request, body included, and would fail only in production.
fn division_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(120))
        .build()
        .expect("failed to build VATUSA HTTP client")
}

async fn fetch_division(api_key: &str) -> Result<ControllersAndRoles, String> {
    division_client()
        .get(format!("{}/v3/division/controllers", vatusa_api_base()))
        .header("x-api-key", api_key)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("VATUSA division pull failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("VATUSA division pull returned an unreadable body: {e}"))
}

/// Fetch and store the division once, then announce any access change. The daily job runs it, and so
/// does the access reset (#795) before it reads anyone's VATUSA roles.
pub async fn pull_division(
    pool: &PgPool,
    api_key: &str,
    events: &crate::realtime::Events,
) -> Result<String, String> {
    let pulled = fetch_division(api_key).await?;
    apply_and_announce(pool, &division_members(pulled), events).await
}

/// Pull the division daily (and on demand: from Background Tasks, or when a verified webhook delivery
/// says the roster changed). Replaces the old 6-hourly reconcile, which refreshed ≤ 800 already-signed-in
/// members a day over v2, one fetch each.
pub fn spawn_division_pull(reg: Arc<JobRegistry>, pool: PgPool, events: crate::realtime::Events) {
    let Some(api_key) = vatusa_api_key() else {
        return;
    };
    tokio::spawn(division_pull_job(reg, move || {
        let (pool, api_key, events) = (pool.clone(), api_key.clone(), events.clone());
        async move {
            let summary = pull_division(&pool, &api_key, &events).await?;
            // Daily is also when a webhook VATUSA dropped, or one whose secret we can no longer
            // decrypt, gets replaced. Its failure is reported but doesn't fail the pull.
            match ensure_webhook(&pool, &api_key).await {
                Ok(()) => Ok(summary),
                Err(e) => Ok(format!("{summary}; webhook: {e}")),
            }
        }
    }));
}

/// The registered, triggerable job around a pull. The pull itself is an argument so tests can register
/// the job without its first tick reaching VATUSA.
async fn division_pull_job<F, Fut>(reg: Arc<JobRegistry>, pull: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    run_interval(
        reg,
        PULL_JOB,
        "Pull every VATUSA controller and role grant, and re-check the division webhook",
        PULL_INTERVAL,
        pull,
    )
    .await;
}

// --- The division webhook (v3, #605) ---

#[derive(Debug, Deserialize)]
struct CreateWebhookResponse {
    secret: String,
}

#[derive(Debug, Deserialize)]
struct WebhookInfo {
    id: i64,
    url: String,
}

/// Where VATUSA delivers to: one division webhook (v3 scopes webhooks to the calling key).
pub fn webhook_url(public_url: &str) -> String {
    format!(
        "{}/api/v1/webhooks/vatusa",
        public_url.trim_end_matches('/')
    )
}

/// Register the division webhook at startup (when configured). Also re-checked after every pull.
pub fn spawn_register_webhook(pool: PgPool) {
    let Some(api_key) = vatusa_api_key() else {
        return;
    };
    tokio::spawn(async move {
        if let Err(e) = ensure_webhook(&pool, &api_key).await {
            tracing::warn!("VATUSA webhook: {e}");
        }
    });
}

/// The longest stretch of an error response kept in a log line: VATUSA's errors are short JSON, and the
/// cap keeps a proxy's HTML error page from flooding the log.
const ERROR_BODY_MAX: usize = 500;

/// `resp` if it succeeded; otherwise an error naming the call, the status **and VATUSA's response
/// body** (#688). `error_for_status` keeps only the status, so a `400` said nothing about what VATUSA
/// objected to.
async fn ok_or_body(resp: reqwest::Response, what: &str) -> Result<reqwest::Response, String> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let body = body.trim();
    let body: String = body.chars().take(ERROR_BODY_MAX).collect();
    Err(format!("{what}: {status}: {body}"))
}

/// Make sure exactly one usable division webhook exists, ours.
///
/// Usable means: stored, decryptable with today's key, pointing at today's URL, and still listed by
/// VATUSA. Otherwise every webhook on VATUSA's side that points at our receiver — the 22 legacy
/// per-facility ones included — is deleted, a fresh one is created, its id is read back from the list
/// (creating returns only the secret, and only once), and the secret is stored encrypted.
///
/// Needs `OIS_PUBLIC_URL` and `OIS_SECRET_KEY`; without either it does nothing and says so — the
/// daily pull still keeps everyone current, just without push notifications.
async fn ensure_webhook(pool: &PgPool, api_key: &str) -> Result<(), String> {
    let Some(public_url) = ois_public_url() else {
        return Err("not registered: OIS_PUBLIC_URL is unset".into());
    };
    let Some(key) = ois_secret_key() else {
        return Err("not registered: OIS_SECRET_KEY is unset or invalid".into());
    };
    let target = webhook_url(&public_url);
    let http = client();
    let base = format!("{}/v3/webhooks", vatusa_api_base());

    let list = |http: reqwest::Client, base: String| async move {
        let resp = http
            .get(&base)
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|e| format!("list webhooks: {e}"))?;
        ok_or_body(resp, "list webhooks")
            .await?
            .json::<Vec<WebhookInfo>>()
            .await
            .map_err(|e| format!("list webhooks: {e}"))
    };
    let listed = list(http.clone(), base.clone()).await?;

    let stored = repo::fetch_webhook(pool)
        .await
        .map_err(|e| format!("load stored webhook: {e}"))?;
    if let Some(row) = &stored
        && row.url == target
        && row.key_version == crate::secrets::KEY_VERSION
        && crate::secrets::decrypt(&key, &row.secret_ciphertext).is_some()
        && row
            .vatusa_id
            .is_some_and(|id| listed.iter().any(|w| w.id == id))
    {
        // Said positively, so a deploy check can look for registration rather than for the absence
        // of a warning — which a run that never reached VATUSA also produces (#688).
        tracing::info!(
            vatusa_id = row.vatusa_id,
            "VATUSA division webhook already registered"
        );
        return Ok(());
    }

    let receiver_prefix = webhook_url(&public_url);
    for stale in listed
        .iter()
        .filter(|w| w.url.starts_with(&receiver_prefix))
    {
        let deleted = match http
            .delete(format!("{base}/{}", stale.id))
            .header("x-api-key", api_key)
            .send()
            .await
        {
            Ok(resp) => ok_or_body(resp, "delete").await.map(drop),
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = deleted {
            tracing::warn!(
                id = stale.id,
                url = stale.url,
                "delete stale VATUSA webhook: {e}"
            );
        }
    }

    let created = http
        .post(&base)
        .header("x-api-key", api_key)
        .json(&serde_json::json!({ "url": target }))
        .send()
        .await
        .map_err(|e| format!("create webhook: {e}"))?;
    let created: CreateWebhookResponse = ok_or_body(created, "create webhook")
        .await?
        .json()
        .await
        .map_err(|e| format!("create webhook: {e}"))?;
    let vatusa_id = list(http, base).await.ok().and_then(|after| {
        after
            .into_iter()
            .filter(|w| w.url == target)
            .map(|w| w.id)
            .max()
    });

    repo::store_webhook(
        pool,
        &repo::StoredWebhook {
            vatusa_id,
            url: target,
            secret_ciphertext: crate::secrets::encrypt(&key, &created.secret),
            key_version: crate::secrets::KEY_VERSION,
        },
    )
    .await
    .map_err(|e| format!("store webhook: {e}"))?;
    tracing::info!(vatusa_id, "VATUSA division webhook registered");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answer one HTTP request with `status` and `body`, from a local socket; returns its URL.
    async fn serve_once(status: &'static str, body: String) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{addr}/v3/webhooks")
    }

    /// #757: the v2 member fetch sends the key as `?apikey=`, and reqwest's error text carries the request
    /// URL. Whatever goes wrong, the error that reaches the sign-in warning log must not hold the key.
    const KEY: &str = "vatusa-secret-key-757";

    #[tokio::test]
    async fn a_failed_member_fetch_does_not_log_the_api_key() {
        let url = serve_once("500 Internal Server Error", String::new()).await;
        let base = url.trim_end_matches("/v3/webhooks");
        let err = fetch_member(&client(), base, KEY, 1_757_000)
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains(KEY), "the key leaked into the error: {err}");
        assert!(
            err.contains("500"),
            "the error should still say what failed: {err}"
        );
    }

    /// A transport failure is an error from `send`, not from `error_for_status`: pin that path too.
    #[tokio::test]
    async fn an_unreachable_vatusa_does_not_log_the_api_key() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let err = fetch_member(&client(), &base, KEY, 1_757_000)
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains(KEY), "the key leaked into the error: {err}");
    }

    /// #688 AC5: a refused call says what VATUSA said, not only the status — a `400` from a bad key
    /// or a bad body is now diagnosable from the log line.
    #[tokio::test]
    async fn a_refused_call_carries_vatusas_response_body() {
        let url = serve_once("400 Bad Request", r#"{"message":"Invalid API key"}"#.into()).await;
        let resp = client().post(&url).send().await.unwrap();
        let err = ok_or_body(resp, "create webhook").await.unwrap_err();
        assert_eq!(
            err,
            r#"create webhook: 400 Bad Request: {"message":"Invalid API key"}"#
        );
    }

    /// A long error page is cut, so a proxy's HTML can't flood the log.
    #[tokio::test]
    async fn a_long_error_body_is_truncated() {
        let url = serve_once("503 Service Unavailable", "x".repeat(5_000)).await;
        let resp = client().get(&url).send().await.unwrap();
        let err = ok_or_body(resp, "list webhooks").await.unwrap_err();
        let body = err.rsplit(": ").next().unwrap();
        assert_eq!(body.len(), ERROR_BODY_MAX);
    }

    /// Success passes through untouched, body and all.
    #[tokio::test]
    async fn a_successful_call_passes_through() {
        let url = serve_once("200 OK", "[]".into()).await;
        let resp = client().get(&url).send().await.unwrap();
        let ok = ok_or_body(resp, "list webhooks").await.unwrap();
        assert_eq!(ok.json::<Vec<WebhookInfo>>().await.unwrap().len(), 0);
    }

    /// [`apply_division`] reports whether access moved and holds no sender, so it cannot announce
    /// anything before its own writes have committed — [`apply_and_announce`] does that after it
    /// returns (#644 review). The flag is what decides the nudge, so pin it on its own.
    #[sqlx::test]
    async fn the_pull_reports_whether_anyones_access_moved(pool: PgPool) {
        map(&pool, "MTR", "EC").await;
        let everyone: Vec<i64> = (1_644_700..1_644_705).collect();
        let roster = || {
            everyone
                .iter()
                .map(|c| controller(*c, "ZDC"))
                .collect::<Vec<_>>()
        };
        let roles = || {
            everyone
                .iter()
                .map(|c| role(*c, "ZDC", "MTR"))
                .collect::<Vec<_>>()
        };

        apply_division(&pool, &pulled(roster(), vec![]))
            .await
            .unwrap();
        let (_, moved) = apply_division(&pool, &pulled(roster(), roles()))
            .await
            .unwrap();
        assert!(moved, "roles were granted");
        let (_, moved) = apply_division(&pool, &pulled(roster(), roles()))
            .await
            .unwrap();
        assert!(!moved, "the same pull again moves nothing");
    }

    /// A realtime hub nothing listens on, for pulls whose nudge a test doesn't check.
    fn hub() -> crate::realtime::Events {
        crate::realtime::Events::new(None)
    }
    use serde_json::json;

    #[test]
    fn short_rating_prefers_field_then_maps_numeric() {
        let m: VatusaMember =
            serde_json::from_value(json!({ "cid": 1, "rating": 5, "rating_short": "C1" })).unwrap();
        assert_eq!(m.short_rating().as_deref(), Some("C1"));
        let m2: VatusaMember = serde_json::from_value(json!({ "cid": 1, "rating": 8 })).unwrap();
        assert_eq!(m2.short_rating().as_deref(), Some("I1"));
    }

    #[test]
    fn de_flag_accepts_bool_or_int() {
        let one: VatusaMember =
            serde_json::from_value(json!({ "cid": 1, "flag_homecontroller": 1 })).unwrap();
        assert!(one.flag_homecontroller);
        let no: VatusaMember =
            serde_json::from_value(json!({ "cid": 1, "flag_homecontroller": false })).unwrap();
        assert!(!no.flag_homecontroller);
    }

    /// AC3 (#548): the login sync is *awaited* — when `completes_within` returns, the mapped grant
    /// already exists, so the session issued next carries it. Driven with an in-process sync rather
    /// than the HTTP fetch, which is the only part replaced.
    #[sqlx::test]
    async fn the_login_sync_has_landed_by_the_time_it_returns(pool: PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name, cid) \
             values ('T', 'T', 1548001) returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.vatusa_role_mappings (vatusa_role, role_name) values ('DATM', 'EC')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let member: VatusaMember = serde_json::from_value(json!({
            "cid": 1548001, "roles": [{ "role": "DATM", "facility": "ZDC" }]
        }))
        .unwrap();

        let sync = async {
            repo::upsert_member(&pool, &member)
                .await
                .map_err(|e| e.to_string())
        };
        assert!(completes_within(LOGIN_SYNC_BUDGET, sync).await);

        let held: Vec<String> = sqlx::query_scalar(
            "select role_name from access.user_roles where user_id = $1 and source = 'vatusa'",
        )
        .bind(&user)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(held, vec!["EC".to_string()]);
    }

    /// A VATUSA that never answers cannot hold sign-in hostage: the budget ends the wait.
    #[tokio::test]
    async fn a_hung_vatusa_is_abandoned_at_the_budget() {
        let started = std::time::Instant::now();
        let hung = std::future::pending::<Result<(), String>>();
        assert!(!completes_within(Duration::from_millis(50), hung).await);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    /// The wiring half of AC3, which a unit test of the helper cannot see: sign-in must await the sync
    /// *before* issuing the session, and must no longer fire-and-forget it. Reverting `auth.rs` to the
    /// detached `spawn_member_sync` fails this.
    #[test]
    fn sign_in_awaits_the_sync_before_issuing_the_session() {
        let auth = include_str!("../handlers/auth.rs");
        let sync = auth
            .find("sync_member_on_login(pool, profile.cid).await")
            .expect("sign-in must await sync_member_on_login");
        let session = auth
            .find("insert_session(")
            .expect("sign-in issues a session");
        assert!(
            sync < session,
            "the sync must land before the session is issued"
        );
        assert!(
            !auth.contains("spawn_member_sync"),
            "sign-in must not detach the VATUSA sync"
        );
    }

    // ---- #605: the division pull ----

    fn controller(cid: i64, facility: &str) -> serde_json::Value {
        json!({
            "cid": cid, "display_name": format!("Controller {cid}"), "controller_rating": 5,
            "instructor_rating": 0, "facility": facility, "visiting_facilities": [],
            "last_competency_date": null, "last_promotion_time": null, "last_transfer_time": null,
        })
    }

    fn role(cid: i64, facility: &str, role: &str) -> serde_json::Value {
        json!({ "id": 1, "cid": cid, "facility": facility, "role": role,
                "grantor_cid": 1, "granted_at": 1_700_000_000 })
    }

    /// A pull as v3 serves it, through serde — the same path the HTTP response takes.
    fn pulled(
        controllers: Vec<serde_json::Value>,
        roles: Vec<serde_json::Value>,
    ) -> Vec<repo::DivisionMember> {
        division_members(
            serde_json::from_value(json!({ "controllers": controllers, "roles": roles })).unwrap(),
        )
    }

    async fn held(pool: &PgPool, cid: i64) -> Vec<(String, Option<String>, String)> {
        sqlx::query_as(
            "select r.role_name, r.artcc_id, r.source from access.user_roles r \
             join identity.users u on u.id = r.user_id where u.cid = $1 \
             order by r.role_name, r.source",
        )
        .bind(cid)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn stored_roles(pool: &PgPool, cid: i64) -> Vec<(String, String)> {
        sqlx::query_as(
            "select facility, role from identity.vatusa_roles where cid = $1 order by facility, role",
        )
        .bind(cid)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn map(pool: &PgPool, vatusa_role: &str, group: &str) {
        sqlx::query(
            "insert into access.vatusa_role_mappings (vatusa_role, role_name) values ($1, $2)",
        )
        .bind(vatusa_role)
        .bind(group)
        .execute(pool)
        .await
        .unwrap();
    }

    /// v3's codes arrive as VATUSA spells them; OIS stores one canonical form. `*` (v3's division-wide
    /// marker) becomes `ZHQ`, so the national mapping case keeps working (AC4).
    #[test]
    fn the_pull_is_normalised_before_it_is_stored() {
        let members = pulled(
            vec![
                controller(3, " zdc "),
                controller(1, "ZNY"),
                controller(1, "ZNY"),
            ],
            vec![
                role(3, "*", " wm "),
                role(3, "zdc", "datm"),
                role(99, "ZDC", "ATM"),
            ],
        );
        assert_eq!(
            members.iter().map(|m| m.cid).collect::<Vec<_>>(),
            [1, 3],
            "sorted, deduped"
        );
        let m = &members[1];
        assert_eq!(m.facility, "ZDC");
        assert_eq!(m.rating_short.as_deref(), Some("C1"));
        let roles: Vec<_> = m
            .roles
            .iter()
            .map(|(f, r, _)| (f.as_str(), r.as_str()))
            .collect();
        assert_eq!(roles, [("ZHQ", "WM"), ("ZDC", "DATM")]);
        assert!(m.roles[0].2.is_some(), "granted_at is unix seconds");
        // A role for a CID not in the pull has no one to attach to.
        assert!(members.iter().all(|m| m.cid != 99));
    }

    /// AC2: every controller is seeded with roles and visits, across several transactions, and running
    /// the same pull again changes nothing — no duplicates, no churn, no audit noise.
    #[sqlx::test]
    async fn the_pull_seeds_the_division_and_a_rerun_is_idempotent(pool: PgPool) {
        let controllers: Vec<_> = (1..=1_200)
            .map(|cid| controller(1_605_000 + cid, "ZDC"))
            .collect();
        let mut with_visits = controllers.clone();
        with_visits[0]["visiting_facilities"] = json!(["zny"]);
        let members = pulled(with_visits, vec![role(1_605_001, "ZDC", "MTR")]);

        let summary = apply_and_announce(&pool, &members, &hub()).await.unwrap();
        assert!(
            summary.starts_with("1200 controllers (1200 new); access changed for 1200;"),
            "{summary}"
        );

        let users: i64 = sqlx::query_scalar(
            "select count(*) from identity.users where cid between 1605001 and 1606200",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(users, 1_200, "three chunks of 500, every controller seeded");
        assert_eq!(
            stored_roles(&pool, 1_605_001).await,
            [("ZDC".into(), "MTR".into())]
        );
        let visit: String =
            sqlx::query_scalar("select facility from identity.vatusa_visits where cid = 1605001")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(visit, "ZNY");

        let again = apply_and_announce(&pool, &members, &hub()).await.unwrap();
        assert!(
            again.starts_with("1200 controllers (0 new); access changed for 0"),
            "{again}"
        );
        let users_after: i64 = sqlx::query_scalar("select count(*) from identity.users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(users_after, users);
    }

    /// A seeded user has never signed in, keeps no audit actor, and gets VATUSA-mapped access before
    /// their first sign-in — a division role (`*`) reaching them as a national grant (AC4).
    #[sqlx::test]
    async fn a_division_role_grants_national_access_before_first_sign_in(pool: PgPool) {
        map(&pool, "WM", "VATUSA_STAFF").await;

        apply_and_announce(
            &pool,
            &pulled(
                vec![controller(1_605_100, "ZHQ")],
                vec![role(1_605_100, "*", "WM")],
            ),
            &hub(),
        )
        .await
        .unwrap();

        assert_eq!(
            held(&pool, 1_605_100).await,
            [("VATUSA_STAFF".to_string(), None, "vatusa".to_string())]
        );
        let (signed_in, actors): (Option<chrono::DateTime<Utc>>, i64) = sqlx::query_as(
            "select u.last_login_at, (select count(*) from access.actors a where a.user_id = u.id) \
             from identity.users u where u.cid = 1605100",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((signed_in, actors), (None, 0));
    }

    /// Losing a VATUSA role in the pull revokes the access mapped from it, and the audit names the role
    /// that was lost — the pull captures what the old roles justified before it rewrites them.
    #[sqlx::test]
    async fn a_role_lost_in_the_pull_revokes_its_access_and_names_it(pool: PgPool) {
        map(&pool, "DATM", "EC").await;
        let present = || vec![controller(1_605_200, "ZDC")];
        apply_and_announce(
            &pool,
            &pulled(present(), vec![role(1_605_200, "ZDC", "DATM")]),
            &hub(),
        )
        .await
        .unwrap();
        // EC from the mapping, and CONTROLLER from the ZDC home (#730).
        assert_eq!(held(&pool, 1_605_200).await.len(), 2);

        apply_and_announce(&pool, &pulled(present(), vec![]), &hub())
            .await
            .unwrap();

        // The mapped EC goes with the role; the roster grant stays, since they're still at ZDC.
        assert_eq!(held(&pool, 1_605_200).await.len(), 1);
        let reason: String = sqlx::query_scalar(
            "select reason from access.audit_logs where actor_id = 'vatusa-sync' \
             order by created_at desc, id desc limit 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            reason,
            "VATUSA sync: revoked EC at ZDC (no longer holds DATM@ZDC)"
        );
    }

    /// A controller missing from a (plausible) pull has left the division: their stored roles go, and
    /// the access mapped from them with it. Everyone still present is untouched.
    #[sqlx::test]
    async fn a_departed_controller_loses_their_roles(pool: PgPool) {
        map(&pool, "DATM", "EC").await;
        let roles = |cids: &[i64]| cids.iter().map(|c| role(*c, "ZDC", "DATM")).collect();
        let everyone: Vec<i64> = (1_605_300..1_605_304).collect();
        apply_and_announce(
            &pool,
            &pulled(
                everyone.iter().map(|c| controller(*c, "ZDC")).collect(),
                roles(&everyone),
            ),
            &hub(),
        )
        .await
        .unwrap();

        let staying = &everyone[1..];
        let summary = apply_and_announce(
            &pool,
            &pulled(
                staying.iter().map(|c| controller(*c, "ZDC")).collect(),
                roles(staying),
            ),
            &hub(),
        )
        .await
        .unwrap();

        assert!(summary.ends_with("1 departed"), "{summary}");
        assert!(stored_roles(&pool, everyone[0]).await.is_empty());
        assert!(held(&pool, everyone[0]).await.is_empty());
        // Still present: their mapped EC, and the roster CONTROLLER their ZDC home grants (#730).
        assert_eq!(held(&pool, everyone[1]).await.len(), 2);
    }

    /// AC5. v3 carries no `discord_id`. The bot resolves DMs through this mapping, which sign-in
    /// refreshes over v2 — the pull must leave it exactly as it found it, because absence of a field is
    /// not a cleared link.
    #[sqlx::test]
    async fn the_pull_never_clears_a_discord_link(pool: PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name, cid, last_login_at) \
             values ('Signed In', 'Signed In', 1605400, now()) returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into integration.external_sync_mappings \
                 (system_code, entity_type, local_id, external_id, metadata) \
             values ('discord', 'user', $1, '123456789012345678', '{\"source\":\"vatusa\"}'::jsonb)",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        apply_and_announce(
            &pool,
            &pulled(vec![controller(1_605_400, "ZDC")], vec![]),
            &hub(),
        )
        .await
        .unwrap();

        let link: Option<String> = sqlx::query_scalar(
            "select external_id from integration.external_sync_mappings \
             where system_code = 'discord' and local_id = $1",
        )
        .bind(&user)
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(link.as_deref(), Some("123456789012345678"));
        let name: String = sqlx::query_scalar("select full_name from identity.users where id = $1")
            .bind(&user)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            name, "Signed In",
            "names are sign-in's to own, from VATSIM Connect"
        );
    }

    /// The home-controller flag shown on the profile: sign-in sets VATUSA's own value over v2, which
    /// the pull can only approximate (home facility is a real ARTCC). The pull fills it for seeded
    /// users and never overrides one sign-in set, or the profile would flip between the two daily.
    #[sqlx::test]
    async fn the_pull_does_not_override_sign_ins_home_controller_flag(pool: PgPool) {
        sqlx::query(
            "insert into identity.users (full_name, display_name, cid, last_login_at, \
                 flag_home_controller) values ('A', 'A', 1605450, now(), true)",
        )
        .execute(&pool)
        .await
        .unwrap();

        apply_and_announce(
            &pool,
            &pulled(
                vec![controller(1_605_450, "ZAE"), controller(1_605_451, "ZDC")],
                vec![],
            ),
            &hub(),
        )
        .await
        .unwrap();

        let flags: Vec<(i64, Option<bool>)> = sqlx::query_as(
            "select cid, flag_home_controller from identity.users \
             where cid in (1605450, 1605451) order by cid",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(flags, [(1_605_450, Some(true)), (1_605_451, Some(true))]);
    }

    /// Absence from the pull strips roles, so a truncated response would be a mass revocation. One that
    /// is empty, or under half the members already synced, is refused before anything is written.
    #[sqlx::test]
    async fn a_truncated_pull_is_refused_without_writing(pool: PgPool) {
        let everyone: Vec<i64> = (1_605_500..1_605_510).collect();
        apply_and_announce(
            &pool,
            &pulled(
                everyone.iter().map(|c| controller(*c, "ZDC")).collect(),
                everyone.iter().map(|c| role(*c, "ZDC", "MTR")).collect(),
            ),
            &hub(),
        )
        .await
        .unwrap();

        let three = pulled(
            everyone[..3]
                .iter()
                .map(|c| controller(*c, "ZDC"))
                .collect(),
            vec![],
        );
        let refused = apply_and_announce(&pool, &three, &hub()).await;
        assert!(refused.is_err_and(|e| e.contains("looks truncated")));
        assert!(apply_and_announce(&pool, &[], &hub()).await.is_err());

        // Nothing was applied: everyone missing from the bad pull still holds their role.
        assert_eq!(
            stored_roles(&pool, everyone[9]).await,
            [("ZDC".into(), "MTR".into())]
        );
        assert_eq!(stored_roles(&pool, everyone[0]).await.len(), 1);
    }

    // ---- #644: a roster change tells signed-in browsers ---------------------------------------------

    fn access_nudges(rx: &mut tokio::sync::broadcast::Receiver<crate::realtime::WsEvent>) -> usize {
        let mut n = 0;
        while let Ok(e) = rx.try_recv() {
            if e.topic == crate::realtime::topic::ACCESS_GRANTED {
                n += 1;
            }
        }
        n
    }

    /// A pull that moves someone's mapped access tells browsers once — and by the time they hear it,
    /// the access is already there to be read.
    #[sqlx::test]
    async fn a_pull_that_changes_access_tells_browsers_once_after_it_lands(pool: PgPool) {
        map(&pool, "MTR", "EC").await;
        let everyone: Vec<i64> = (1_644_600..1_644_610).collect();
        let roster = || everyone.iter().map(|c| controller(*c, "ZDC")).collect();
        let events = crate::realtime::Events::new(None);
        let mut rx = events.subscribe();

        apply_and_announce(&pool, &pulled(roster(), vec![]), &events)
            .await
            .unwrap();
        access_nudges(&mut rx); // the first sync of a fresh roster is not what this test is about

        let roles = everyone.iter().map(|c| role(*c, "ZDC", "MTR")).collect();
        apply_and_announce(&pool, &pulled(roster(), roles), &events)
            .await
            .unwrap();
        assert_eq!(access_nudges(&mut rx), 1, "one nudge for the whole pull");
        assert_eq!(
            held(&pool, everyone[0]).await.len(),
            2,
            "and the access it announces is already stored (the MTR mapping, beside the roster grant)"
        );
    }

    #[sqlx::test]
    async fn a_pull_that_changes_nothing_tells_no_one(pool: PgPool) {
        map(&pool, "MTR", "EC").await;
        let everyone: Vec<i64> = (1_644_700..1_644_710).collect();
        let roster = || everyone.iter().map(|c| controller(*c, "ZDC")).collect();
        let roles = || everyone.iter().map(|c| role(*c, "ZDC", "MTR")).collect();
        let events = crate::realtime::Events::new(None);
        let mut rx = events.subscribe();
        apply_and_announce(&pool, &pulled(roster(), roles()), &events)
            .await
            .unwrap();
        access_nudges(&mut rx);

        apply_and_announce(&pool, &pulled(roster(), roles()), &events)
            .await
            .unwrap();
        assert_eq!(access_nudges(&mut rx), 0);
    }

    /// Leaving the division removes mapped access, so it is a change worth announcing.
    #[sqlx::test]
    async fn a_departure_tells_browsers(pool: PgPool) {
        map(&pool, "MTR", "EC").await;
        let everyone: Vec<i64> = (1_644_800..1_644_810).collect();
        let roster = |who: &[i64]| who.iter().map(|c| controller(*c, "ZDC")).collect();
        let roles = |who: &[i64]| who.iter().map(|c| role(*c, "ZDC", "MTR")).collect();
        let events = crate::realtime::Events::new(None);
        let mut rx = events.subscribe();
        apply_and_announce(&pool, &pulled(roster(&everyone), roles(&everyone)), &events)
            .await
            .unwrap();
        access_nudges(&mut rx);

        let stayed = &everyone[..9];
        apply_and_announce(&pool, &pulled(roster(stayed), roles(stayed)), &events)
            .await
            .unwrap();
        assert_eq!(access_nudges(&mut rx), 1);
        assert!(held(&pool, everyone[9]).await.is_empty());
    }

    /// A pull refused as truncated changes nothing, so it tells no one.
    #[sqlx::test]
    async fn a_refused_pull_tells_no_one(pool: PgPool) {
        map(&pool, "MTR", "EC").await;
        let everyone: Vec<i64> = (1_644_900..1_644_910).collect();
        let roster = |who: &[i64]| who.iter().map(|c| controller(*c, "ZDC")).collect();
        let roles = |who: &[i64]| who.iter().map(|c| role(*c, "ZDC", "MTR")).collect();
        let events = crate::realtime::Events::new(None);
        let mut rx = events.subscribe();
        apply_and_announce(&pool, &pulled(roster(&everyone), roles(&everyone)), &events)
            .await
            .unwrap();
        access_nudges(&mut rx);

        let cut = &everyone[..2];
        assert!(
            apply_and_announce(&pool, &pulled(roster(cut), roles(cut)), &events)
                .await
                .is_err()
        );
        assert_eq!(access_nudges(&mut rx), 0);
    }

    /// `roles` is its own array: a pull with every controller but a lost or cut-off role list passes
    /// the controller check, and applied it would revoke the mapped access of the whole division.
    #[sqlx::test]
    async fn a_pull_with_a_truncated_role_list_is_refused(pool: PgPool) {
        map(&pool, "MTR", "EC").await;
        let everyone: Vec<i64> = (1_605_600..1_605_610).collect();
        let roster = || everyone.iter().map(|c| controller(*c, "ZDC")).collect();
        let all_roles = || everyone.iter().map(|c| role(*c, "ZDC", "MTR")).collect();
        apply_and_announce(&pool, &pulled(roster(), all_roles()), &hub())
            .await
            .unwrap();

        for cut in [
            vec![],
            everyone[..4]
                .iter()
                .map(|c| role(*c, "ZDC", "MTR"))
                .collect(),
        ] {
            let refused = apply_and_announce(&pool, &pulled(roster(), cut), &hub()).await;
            assert!(refused.is_err_and(|e| e.contains("role list looks truncated")));
        }
        for cid in &everyone {
            // The mapped grant and the roster grant (#730).
            assert_eq!(held(&pool, *cid).await.len(), 2, "{cid} kept their access");
        }

        // The floor is half: losing a few roles is a real change, and applies.
        let most: Vec<_> = everyone[..6]
            .iter()
            .map(|c| role(*c, "ZDC", "MTR"))
            .collect();
        apply_and_announce(&pool, &pulled(roster(), most), &hub())
            .await
            .unwrap();
        // Their mapped grant went with the role; the roster grant stays, since they're still at ZDC.
        assert_eq!(held(&pool, everyone[9]).await.len(), 1);
    }

    /// The one remaining v2 call is sign-in's (AC8). Counted across the whole backend source, so a
    /// second one — or the old per-member reconcile coming back — fails here.
    #[test]
    fn exactly_one_v2_call_remains() {
        let needle = concat!("/v", "2/");
        fn walk(dir: &std::path::Path, needle: &str, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, needle, hits);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (n, line) in text.lines().enumerate() {
                        // Calls, not mentions: a doc comment naming the path is not a request.
                        if line.contains(needle) && !line.trim_start().starts_with("//") {
                            hits.push(format!("{}:{}", path.display(), n + 1));
                        }
                    }
                }
            }
        }
        let mut hits = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            needle,
            &mut hits,
        );
        assert_eq!(
            hits.len(),
            1,
            "expected only sign-in's fetch_member: {hits:?}"
        );
        assert!(hits[0].contains("feed/vatusa.rs"), "{hits:?}");
    }

    /// The division pull is a registered, triggerable job (#605 AC1): listed on Background Tasks and
    /// runnable on demand there or by a webhook delivery. Registered here with a stand-in pull, so its
    /// immediate first tick never reaches VATUSA.
    #[tokio::test]
    async fn the_division_pull_is_a_triggerable_background_task() {
        let reg = Arc::new(JobRegistry::new());
        let job = tokio::spawn(division_pull_job(reg.clone(), || async {
            Ok("stand-in".to_string())
        }));
        let mut listed = None;
        for _ in 0..100 {
            listed = reg.snapshot().into_iter().find(|j| j.name == PULL_JOB);
            if listed.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let listed = listed.expect("the division pull must be registered");
        assert!(listed.triggerable, "it must be runnable on demand");
        assert!(reg.trigger(PULL_JOB));
        job.abort();
    }
}
