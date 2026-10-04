//! VATUSA member sync. On sign-in we fetch a member's details from the VATUSA API (v2) and
//! store them; to keep them fresh we register a per-facility outbound webhook (v3) and refetch
//! any of our users whose roster changes. A periodic reconciliation backstops the webhooks,
//! which are delivered once with no retries.
//!
//! Everything here no-ops unless `VATUSA_API_KEY` is configured; webhook registration
//! additionally requires `OIS_PUBLIC_URL` (the receiver must be a public HTTPS endpoint).

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};
use sqlx::PgPool;

use crate::config::{ois_public_url, vatusa_api_base, vatusa_api_key};
use crate::job_registry::{JobRegistry, run_interval};
use crate::repos::vatusa as repo;

/// How often the reconciliation job refreshes the least-recently-synced members.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// Members refreshed per reconciliation tick (keeps VATUSA request volume modest).
const RECONCILE_BATCH: i64 = 200;
/// The reconcile job's name on the admin Background Tasks page.
pub const RECONCILE_JOB: &str = "vatusa_reconcile";
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

/// Fetch one member's details from the VATUSA v2 API. `api_key` is passed as `?apikey=` so
/// staff-only fields (email, visits) are populated.
async fn fetch_member(
    http: &reqwest::Client,
    api_key: &str,
    cid: i64,
) -> Result<VatusaMember, reqwest::Error> {
    let url = format!("{}/v2/user/{cid}", vatusa_api_base());
    let env: Envelope<VatusaMember> = http
        .get(&url)
        .query(&[("apikey", api_key)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(env.data)
}

/// Fetch a member and persist their details/roles/visits. The reusable unit shared by the
/// sign-in hook, the webhook receiver, and reconciliation. Best-effort: errors are returned
/// for the caller to log, never to fail the surrounding flow.
pub async fn sync_member(pool: &PgPool, cid: i64) -> Result<(), String> {
    let Some(api_key) = vatusa_api_key() else {
        return Ok(()); // sync disabled
    };
    let member = fetch_member(&client(), &api_key, cid)
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

/// Fire-and-forget a member sync (the webhook receiver, and sign-in's fallback when VATUSA is slow).
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

// --- Webhook registration (v3) ---

#[derive(Debug, Deserialize)]
struct CreateWebhookResponse {
    id: i64,
    secret: String,
}

/// Register (once) a per-facility webhook pointing at our receiver, persisting each returned
/// secret. Runs at startup; skips facilities already registered in `identity.vatusa_webhooks`.
pub fn spawn_register_webhooks(pool: PgPool) {
    let (Some(api_key), Some(public_url)) = (vatusa_api_key(), ois_public_url()) else {
        tracing::info!("VATUSA webhooks not registered (VATUSA_API_KEY / OIS_PUBLIC_URL unset)");
        return;
    };
    tokio::spawn(async move {
        if let Err(e) = register_webhooks(&pool, &api_key, &public_url).await {
            tracing::warn!("VATUSA webhook registration failed: {e}");
        }
    });
}

async fn register_webhooks(pool: &PgPool, api_key: &str, public_url: &str) -> Result<(), String> {
    let facilities = repo::active_facilities(pool)
        .await
        .map_err(|e| format!("load facilities: {e}"))?;
    let existing = repo::registered_facilities(pool)
        .await
        .map_err(|e| format!("load registered webhooks: {e}"))?;
    let http = client();
    let base = format!("{}/v3/webhooks", vatusa_api_base());
    let mut created = 0;
    for facility in facilities {
        if existing.contains(&facility) {
            continue;
        }
        let url = format!("{public_url}/api/v1/webhooks/vatusa/{facility}");
        let resp = http
            .post(&base)
            .header("x-api-key", api_key)
            .json(&serde_json::json!({ "url": url, "facility": facility }))
            .send()
            .await
            .map_err(|e| format!("create webhook for {facility}: {e}"))?;
        if !resp.status().is_success() {
            tracing::warn!(
                "VATUSA webhook create for {facility} returned {}",
                resp.status()
            );
            continue;
        }
        let body: CreateWebhookResponse = resp
            .json()
            .await
            .map_err(|e| format!("parse webhook response for {facility}: {e}"))?;
        repo::upsert_webhook(pool, &facility, body.id, &body.secret, &url)
            .await
            .map_err(|e| format!("persist webhook for {facility}: {e}"))?;
        created += 1;
    }
    tracing::info!(created, "VATUSA webhooks registered");
    Ok(())
}

// --- Reconciliation ---

/// Periodically refresh the least-recently-synced members — a backstop for missed webhook
/// deliveries (which are single-attempt, no-retry) and for non-roster changes (rating, name)
/// that don't emit a webhook.
pub fn spawn_reconcile(reg: Arc<JobRegistry>, pool: PgPool) {
    if vatusa_api_key().is_none() {
        return;
    }
    tokio::spawn(reconcile_job(reg, pool));
}

/// The reconcile loop as a registered job (#548): it appears on the admin Background Tasks page, can
/// be triggered from there, and a tick in which any member failed shows as a failure — before, it ran
/// on a bare interval and a wedged sync was visible only in the logs.
async fn reconcile_job(reg: Arc<JobRegistry>, pool: PgPool) {
    run_interval(
        reg,
        RECONCILE_JOB,
        "Refresh the least-recently-synced VATUSA members and their mapped access",
        RECONCILE_INTERVAL,
        move || {
            let pool = pool.clone();
            async move { reconcile_once(&pool).await }
        },
    )
    .await;
}

async fn reconcile_once(pool: &PgPool) -> Result<String, String> {
    let cids = repo::stale_member_cids(pool, RECONCILE_BATCH)
        .await
        .map_err(|e| e.to_string())?;
    let total = cids.len();
    let mut failed = 0;
    for cid in cids {
        if let Err(e) = sync_member(pool, cid).await {
            tracing::warn!("{e}");
            failed += 1;
        }
    }
    if failed > 0 {
        Err(format!("{failed} of {total} members failed to sync"))
    } else {
        Ok(format!("{total} members refreshed"))
    }
}

/// Extract the affected CIDs from a `roster_change` webhook payload. For `controllers` rows the
/// CID is the primary key; for `visits` rows it's carried in the value snapshots.
pub fn changed_cids(data: &serde_json::Value) -> HashSet<i64> {
    let mut cids = HashSet::new();
    let table = data
        .get("table_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if table == "controllers"
        && let Some(pk) = data.get("row_pk").and_then(serde_json::Value::as_i64)
    {
        cids.insert(pk);
    }
    for key in ["old_value", "new_value"] {
        if let Some(cid) = data
            .get(key)
            .and_then(|v| v.get("cid"))
            .and_then(serde_json::Value::as_i64)
        {
            cids.insert(cid);
        }
    }
    cids
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changed_cids_controllers_uses_row_pk() {
        let data = json!({
            "table_name": "controllers", "operation": "UPDATE", "row_pk": 800007,
            "old_value": { "facility": "ZAE" }, "new_value": { "facility": "ZDC" }
        });
        assert!(changed_cids(&data).contains(&800007));
    }

    #[test]
    fn changed_cids_visits_uses_cid_not_row_pk() {
        let data = json!({
            "table_name": "visits", "operation": "INSERT", "row_pk": 42,
            "old_value": null, "new_value": { "cid": 1234567, "facility": "ZDC" }
        });
        let cids = changed_cids(&data);
        assert!(cids.contains(&1234567));
        assert!(!cids.contains(&42)); // row_pk is the visit id here, not a CID
    }

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

    /// AC7 (#548): the reconcile loop is a registered job, so it is listed on Background Tasks.
    #[sqlx::test]
    async fn the_reconcile_job_is_on_the_background_tasks_page(pool: PgPool) {
        let reg = Arc::new(JobRegistry::new());
        let job = tokio::spawn(reconcile_job(reg.clone(), pool));
        let mut listed = false;
        for _ in 0..100 {
            if reg.snapshot().iter().any(|j| j.name == RECONCILE_JOB) {
                listed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        job.abort();
        assert!(listed, "{RECONCILE_JOB} must be registered");
    }
}
