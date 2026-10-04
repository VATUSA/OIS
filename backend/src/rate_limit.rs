//! Per-caller request rate limiting for the `/api/` surface (#588).
//!
//! Every `/api/` request draws from one bucket chosen by who is calling: an API key or service account
//! by its id, a signed-in user (web cookie or desktop token) by user id, and anyone else by client IP.
//! The caller is already resolved by `auth::middleware::resolve_current_user`, which runs first, so
//! choosing the bucket costs no query. A caller over its allowance gets `429` with `Retry-After`;
//! every limited response carries `RateLimit-Limit` / `RateLimit-Remaining` / `RateLimit-Reset`.
//!
//! The buckets live in this process, so with several backend replicas each enforces its own allowance.

use std::{
    collections::HashMap,
    net::IpAddr,
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{
    Quota, RateLimiter, clock::Clock, middleware::StateInformationMiddleware,
    state::keyed::DefaultKeyedStateStore,
};
use http::{HeaderMap, HeaderName, HeaderValue, header::RETRY_AFTER};

use crate::{
    auth::context::{CurrentApiKey, CurrentServiceAccount, CurrentUser},
    config::rate_limit_per_min,
    errors::ApiError,
};

pub const LIMIT_HEADER: HeaderName = HeaderName::from_static("ratelimit-limit");
pub const REMAINING_HEADER: HeaderName = HeaderName::from_static("ratelimit-remaining");
pub const RESET_HEADER: HeaderName = HeaderName::from_static("ratelimit-reset");

/// Whose allowance a request draws from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Caller {
    ApiKey(String),
    ServiceAccount(String),
    User(String),
    /// Unauthenticated, by client IP. `None` (no proxy header, e.g. local dev) is one shared bucket.
    Ip(Option<IpAddr>),
}

type Limiter = RateLimiter<
    Caller,
    DefaultKeyedStateStore<Caller>,
    governor::clock::DefaultClock,
    StateInformationMiddleware,
>;

fn limiter(per_minute: NonZeroU32) -> Limiter {
    RateLimiter::keyed(Quota::per_minute(per_minute))
        .with_middleware::<StateInformationMiddleware>()
}

/// Requests and refusals per credential since the last flush (#611).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub requests: u64,
    pub refused: u64,
}

/// One credential's counts, as [`RateLimits::take_usage`] hands them to the flush job: `kind` is
/// `api_key` or `service_account`, matching `access.credential_usage.kind`.
#[derive(Debug, PartialEq, Eq)]
pub struct CredentialUsage {
    pub kind: &'static str,
    pub credential_id: String,
    pub usage: Usage,
}

/// The three allowances, each a full minute's worth available as a burst.
pub struct RateLimits {
    /// Integrations: API keys and service accounts without an override.
    credential: Arc<Limiter>,
    /// One limiter per distinct per-credential override (#611), created on first use. Keyed by the
    /// override's value, so every credential overridden to the same number shares a limiter — each
    /// still with its own bucket.
    overrides: Mutex<HashMap<NonZeroU32, Arc<Limiter>>>,
    /// First-party sessions. Generous — a desktop user with several pop-outs polls ~100–150/min.
    user: Arc<Limiter>,
    /// Unauthenticated callers, by IP. A signed-out map tab polls ~15/min.
    anonymous: Arc<Limiter>,
    /// Credential requests and 429s since the last flush (#611). Sessions and IPs aren't counted.
    usage: Mutex<HashMap<Caller, Usage>>,
    /// Inbound webhook deliveries that **failed** (a 4xx: bad signature, unknown facility), by IP, at
    /// the anonymous allowance. A genuine delivery is never charged; see [`enforce`].
    webhook_failures: Limiter,
    /// IPs whose webhook failures are spent, refused until the given instant without reaching the
    /// handler, whose first act is a database read.
    webhook_blocked: Mutex<HashMap<Option<IpAddr>, Instant>>,
}

impl RateLimits {
    pub fn new(credential: NonZeroU32, user: NonZeroU32, anonymous: NonZeroU32) -> Self {
        Self {
            credential: Arc::new(limiter(credential)),
            overrides: Mutex::new(HashMap::new()),
            user: Arc::new(limiter(user)),
            anonymous: Arc::new(limiter(anonymous)),
            usage: Mutex::new(HashMap::new()),
            webhook_failures: limiter(anonymous),
            webhook_blocked: Mutex::new(HashMap::new()),
        }
    }

    pub fn from_env() -> Self {
        Self::new(
            rate_limit_per_min("RATE_LIMIT_CREDENTIAL_PER_MIN", 300),
            rate_limit_per_min("RATE_LIMIT_USER_PER_MIN", 600),
            rate_limit_per_min("RATE_LIMIT_ANON_PER_MIN", 120),
        )
    }

    /// The bucket for this request. A credential wins over a session, and an unrecognised token is
    /// simply unauthenticated — so a junk bearer cannot buy a fresh bucket.
    fn bucket(&self, request: &Request) -> (Arc<Limiter>, Caller) {
        let extensions = request.extensions();
        if let Some(Some(key)) = extensions.get::<Option<CurrentApiKey>>() {
            let limiter = self.credential_limiter(key.rate_limit_per_min);
            return (limiter, Caller::ApiKey(key.id.clone()));
        }
        if let Some(Some(account)) = extensions.get::<Option<CurrentServiceAccount>>() {
            let limiter = self.credential_limiter(account.rate_limit_per_min);
            return (limiter, Caller::ServiceAccount(account.id.clone()));
        }
        if let Some(Some(user)) = extensions.get::<Option<CurrentUser>>() {
            return (self.user.clone(), Caller::User(user.id.clone()));
        }
        (self.anonymous.clone(), Caller::Ip(client_ip(request)))
    }

    /// The limiter for a credential: its override's, or the deployment default when it has none.
    fn credential_limiter(&self, override_per_min: Option<i32>) -> Arc<Limiter> {
        let Some(per_min) = override_per_min
            .and_then(|n| u32::try_from(n).ok())
            .and_then(NonZeroU32::new)
        else {
            return self.credential.clone();
        };
        match self.overrides.lock() {
            Ok(mut overrides) => overrides
                .entry(per_min)
                .or_insert_with(|| Arc::new(limiter(per_min)))
                .clone(),
            Err(_) => self.credential.clone(),
        }
    }

    /// Count a credential's request, and whether it was refused. Other callers aren't counted.
    fn record(&self, caller: &Caller, refused: bool) {
        if !matches!(caller, Caller::ApiKey(_) | Caller::ServiceAccount(_)) {
            return;
        }
        if let Ok(mut usage) = self.usage.lock() {
            let counts = usage.entry(caller.clone()).or_default();
            counts.requests += 1;
            counts.refused += u64::from(refused);
        }
    }

    /// Every credential's counts since the last call, emptying them — the flush job's input.
    pub fn take_usage(&self) -> Vec<CredentialUsage> {
        let drained = match self.usage.lock() {
            Ok(mut usage) => std::mem::take(&mut *usage),
            Err(_) => return Vec::new(),
        };
        drained
            .into_iter()
            .filter_map(|(caller, usage)| {
                let (kind, credential_id) = match caller {
                    Caller::ApiKey(id) => ("api_key", id),
                    Caller::ServiceAccount(id) => ("service_account", id),
                    _ => return None,
                };
                Some(CredentialUsage {
                    kind,
                    credential_id,
                    usage,
                })
            })
            .collect()
    }

    /// Forget callers whose bucket has refilled, so the maps don't grow with every IP ever seen.
    fn retain_recent(&self) {
        self.credential.retain_recent();
        if let Ok(overrides) = self.overrides.lock() {
            overrides.values().for_each(|l| l.retain_recent());
        }
        self.user.retain_recent();
        self.anonymous.retain_recent();
        self.webhook_failures.retain_recent();
        let now = Instant::now();
        if let Ok(mut blocked) = self.webhook_blocked.lock() {
            blocked.retain(|_, until| *until > now);
        }
    }

    /// How long `ip` is still refused for having spent its webhook failures, if it is.
    fn webhook_block(&self, ip: Option<IpAddr>) -> Option<Duration> {
        let blocked = self.webhook_blocked.lock().ok()?;
        let until = *blocked.get(&ip)?;
        until.checked_duration_since(Instant::now())
    }

    /// Charge a failed delivery to its sender; once their failures are spent, refuse them until a
    /// failure's worth has refilled.
    fn charge_webhook_failure(&self, ip: Option<IpAddr>) {
        if let Err(denied) = self.webhook_failures.check_key(&Caller::Ip(ip)) {
            let wait = denied.wait_time_from(self.webhook_failures.clock().now());
            if let Ok(mut blocked) = self.webhook_blocked.lock() {
                blocked.insert(ip, Instant::now() + wait);
            }
        }
    }
}

/// A per-credential override as an admin sends it (#611): `None` clears it; anything but a positive
/// whole number is refused rather than stored.
pub fn validate_override(per_min: Option<i32>) -> Result<Option<i32>, ApiError> {
    match per_min {
        Some(n) if n <= 0 => Err(ApiError::BadRequest),
        other => Ok(other),
    }
}

/// The caller's address, as the anonymous bucket keys it.
fn client_ip(request: &Request) -> Option<IpAddr> {
    crate::repos::audit::client_ip(request.headers()).and_then(|ip| ip.parse().ok())
}

/// Prune idle buckets once a minute — a full bucket is indistinguishable from a forgotten one.
pub fn spawn_cleanup(limits: Arc<RateLimits>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            limits.retain_recent();
        }
    });
}

/// Inbound, HMAC-authenticated webhooks: genuine deliveries are never rate-limited, failed ones are
/// (see [`enforce`]).
const WEBHOOK_PREFIX: &str = "/api/v1/webhooks/";

/// Middleware: charge the request to its caller's bucket, or refuse it with `429`.
pub async fn enforce(
    State(limits): State<Arc<RateLimits>>,
    request: Request,
    next: Next,
) -> Response {
    // Health, metrics and the OpenAPI docs stay reachable whatever a caller has spent.
    if !request.uri().path().starts_with("/api/") {
        return next.run(request).await;
    }
    // Inbound webhooks are not clients and cannot back off: VATUSA delivers each roster change once,
    // with no retry (`feed/vatusa.rs`), and every delivery arrives from the same servers.
    // Charged to the anonymous IP bucket, a bulk roster change would lose deliveries silently — and
    // after #548 those deliveries drive access. So a genuine delivery is never charged (#588 review).
    //
    // A *failed* one is. The handler reads the webhook's secret from the database before it can
    // verify anything, so an uncharged forged flood would be unlimited database load. Failures (a 4xx)
    // are charged to the sender's IP at the anonymous allowance, and once those are spent the sender is
    // refused here, before the handler, until they refill. A server-side 5xx is not the sender's fault
    // and is not charged.
    if request.uri().path().starts_with(WEBHOOK_PREFIX) {
        let ip = client_ip(&request);
        if let Some(wait) = limits.webhook_block(ip) {
            let mut response = ApiError::TooManyRequests.into_response();
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(whole_seconds(wait).max(1)));
            return response;
        }
        let response = next.run(request).await;
        if response.status().is_client_error() {
            limits.charge_webhook_failure(ip);
        }
        return response;
    }
    let (limiter, caller) = limits.bucket(&request);
    let checked = limiter.check_key(&caller);
    limits.record(&caller, checked.is_err());
    match checked {
        Ok(state) => {
            let quota = state.quota();
            let remaining = state.remaining_burst_capacity();
            let mut response = next.run(request).await;
            let refill = quota.replenish_interval() * (quota.burst_size().get() - remaining);
            set_headers(response.headers_mut(), quota, remaining, refill);
            response
        }
        Err(denied) => {
            let quota = denied.quota();
            let wait = denied.wait_time_from(limiter.clock().now());
            let refill = wait + quota.replenish_interval() * (quota.burst_size().get() - 1);
            let mut response = ApiError::TooManyRequests.into_response();
            let headers = response.headers_mut();
            set_headers(headers, quota, 0, refill);
            headers.insert(RETRY_AFTER, HeaderValue::from(whole_seconds(wait).max(1)));
            response
        }
    }
}

/// `RateLimit-Reset` is the seconds until the full allowance is back.
fn set_headers(headers: &mut HeaderMap, quota: Quota, remaining: u32, refill: Duration) {
    headers.insert(LIMIT_HEADER, HeaderValue::from(quota.burst_size().get()));
    headers.insert(REMAINING_HEADER, HeaderValue::from(remaining));
    headers.insert(RESET_HEADER, HeaderValue::from(whole_seconds(refill)));
}

/// Rounded up: a client told "0" would retry immediately and be refused again.
fn whole_seconds(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        router::build_router_with_limits,
        scope_test_support::{seed_user, session_cookie, test_state},
        state::AppState,
    };
    use axum::{Router, body::Body};
    use sqlx::PgPool;
    use tower::ServiceExt;

    const TRAFFIC: &str = "/api/v1/flow/traffic";

    /// A router whose every bucket holds two requests. Refill is 30 s per request, so nothing refills
    /// mid-test.
    fn router(state: AppState) -> Router {
        let two = NonZeroU32::new(2).unwrap();
        build_router_with_limits(state, Arc::new(RateLimits::new(two, two, two)))
    }

    async fn call(router: &Router, uri: &str, headers: &[(&str, &str)]) -> Response {
        let mut request = http::Request::builder().uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn header(response: &Response, name: &str) -> String {
        response.headers()[name].to_str().unwrap().to_string()
    }

    /// AC1: past its allowance a caller is refused with 429 and told how long to wait, and every
    /// response shows how much is left. AC2/AC5: a second IP is unaffected.
    #[tokio::test]
    async fn a_spent_allowance_is_refused_with_retry_after_and_others_are_unaffected() {
        let router = router(AppState::without_db());
        let a = [("x-forwarded-for", "203.0.113.1")];

        let first = call(&router, TRAFFIC, &a).await;
        assert_eq!(first.status(), http::StatusCode::OK);
        assert_eq!(header(&first, "ratelimit-limit"), "2");
        assert_eq!(header(&first, "ratelimit-remaining"), "1");
        assert_eq!(
            call(&router, TRAFFIC, &a).await.status(),
            http::StatusCode::OK
        );

        let refused = call(&router, TRAFFIC, &a).await;
        assert_eq!(refused.status(), http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(header(&refused, "ratelimit-remaining"), "0");
        let retry_after: u64 = header(&refused, "retry-after").parse().unwrap();
        assert!(
            (1..=30).contains(&retry_after),
            "one request refills in 30 s: {retry_after}"
        );
        let body = axum::body::to_bytes(refused.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body[..], br#"{"error":"too_many_requests"}"#);

        let b = [("x-forwarded-for", "203.0.113.2")];
        assert_eq!(
            call(&router, TRAFFIC, &b).await.status(),
            http::StatusCode::OK
        );
    }

    /// AC2: each API key and each signed-in user has its own allowance, so an anonymous flood from
    /// the same address — or another credential's — does not spend theirs.
    #[sqlx::test]
    async fn credentials_sessions_and_anonymous_callers_draw_from_separate_buckets(pool: PgPool) {
        let user = seed_user(&pool).await;
        let (token, other_token) = ("ois_pat_ratelimit-one", "ois_pat_ratelimit-two");
        for (name, secret) in [("one", token), ("two", other_token)] {
            sqlx::query(
                "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
                 values ($1, $2, 'ois_pat_rate', $3)",
            )
            .bind(&user)
            .bind(name)
            .bind(crate::repos::access::sha256_hex(secret))
            .execute(&pool)
            .await
            .unwrap();
        }
        let cookie = session_cookie(&pool, &user).await;
        let router = router(test_state(pool, Default::default()));
        let ip = ("x-forwarded-for", "203.0.113.1");
        let bearer = format!("Bearer {token}");
        let key = [ip, ("authorization", bearer.as_str())];
        let session = [ip, ("cookie", cookie.as_str())];

        for _ in 0..2 {
            assert_eq!(
                call(&router, TRAFFIC, &[ip]).await.status(),
                http::StatusCode::OK
            );
        }
        assert_eq!(
            call(&router, TRAFFIC, &[ip]).await.status(),
            http::StatusCode::TOO_MANY_REQUESTS,
            "the anonymous bucket for this IP is spent"
        );

        for _ in 0..2 {
            assert_eq!(
                call(&router, TRAFFIC, &key).await.status(),
                http::StatusCode::OK
            );
        }
        assert_eq!(
            call(&router, TRAFFIC, &key).await.status(),
            http::StatusCode::TOO_MANY_REQUESTS,
            "the key has its own allowance, and spends it"
        );
        let other_bearer = format!("Bearer {other_token}");
        assert_eq!(
            call(
                &router,
                TRAFFIC,
                &[ip, ("authorization", other_bearer.as_str())]
            )
            .await
            .status(),
            http::StatusCode::OK,
            "and only its own: another key of the same owner is still served"
        );

        assert_eq!(
            call(&router, TRAFFIC, &session).await.status(),
            http::StatusCode::OK,
            "the signed-in user is still served"
        );
    }

    /// Counts row updates on both credential tables, so a test can see writes a refused request causes.
    async fn count_credential_writes(pool: &PgPool) {
        sqlx::raw_sql(
            "create table public.credential_writes (tbl text); \
             create function public.note_credential_write() returns trigger language plpgsql as \
               $$ begin insert into public.credential_writes values (tg_table_name); return new; end $$; \
             create trigger note_write after update on access.api_keys \
               for each row execute function public.note_credential_write(); \
             create trigger note_write after update on access.service_account_credentials \
               for each row execute function public.note_credential_write();",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn credential_writes(pool: &PgPool, table: &str) -> i64 {
        sqlx::query_scalar("select count(*) from public.credential_writes where tbl = $1")
            .bind(table)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// #588 AC5. The caller is resolved before the limiter can refuse, and resolving a credential stamps
    /// its `last_used_at`. Unthrottled, a key polling far over its limit wrote its row on every refused
    /// request — load on the pool everyone shares, which the limit exists to prevent.
    #[sqlx::test]
    async fn a_credential_over_its_limit_does_not_write_on_every_refused_request(pool: PgPool) {
        let user = seed_user(&pool).await;
        let key_token = "ois_pat_flood-key";
        sqlx::query(
            "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
             values ($1, 'flood', 'ois_pat_floo', $2)",
        )
        .bind(&user)
        .bind(crate::repos::access::sha256_hex(key_token))
        .execute(&pool)
        .await
        .unwrap();
        let sa_token = "ois_sa_flood-account";
        crate::repos::service_accounts::create_service_account(
            &pool,
            "flood",
            "Flood",
            None,
            &crate::repos::access::sha256_hex(sa_token),
        )
        .await
        .unwrap();
        count_credential_writes(&pool).await;
        let router = router(test_state(pool.clone(), Default::default()));

        for (token, table) in [
            (key_token, "api_keys"),
            (sa_token, "service_account_credentials"),
        ] {
            let bearer = format!("Bearer {token}");
            let headers = [
                ("x-forwarded-for", "203.0.113.1"),
                ("authorization", bearer.as_str()),
            ];
            let mut refused = 0;
            for _ in 0..12 {
                if call(&router, TRAFFIC, &headers).await.status()
                    == http::StatusCode::TOO_MANY_REQUESTS
                {
                    refused += 1;
                }
            }
            assert_eq!(
                refused, 10,
                "{table}: the bucket of two is spent, the rest refused"
            );
            assert_eq!(
                credential_writes(&pool, table).await,
                1,
                "{table}: twelve requests in a minute stamp last-used once, not once per request"
            );
        }
    }

    /// The other side of the once-a-minute throttle: a first use is recorded, and a stamp older than a
    /// minute is refreshed — "last used" must not freeze at the first request ever made.
    #[sqlx::test]
    async fn last_used_is_recorded_on_first_use_and_refreshed_once_a_minute_has_passed(
        pool: PgPool,
    ) {
        let user = seed_user(&pool).await;
        let token = "ois_pat_lastused";
        let key_id: String = sqlx::query_scalar(
            "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
             values ($1, 'lastused', 'ois_pat_last', $2) returning id",
        )
        .bind(&user)
        .bind(crate::repos::access::sha256_hex(token))
        .fetch_one(&pool)
        .await
        .unwrap();
        let sa_token = "ois_sa_lastused";
        let sa_id = crate::repos::service_accounts::create_service_account(
            &pool,
            "lastused",
            "Last used",
            None,
            &crate::repos::access::sha256_hex(sa_token),
        )
        .await
        .unwrap();
        let router = router(test_state(pool.clone(), Default::default()));
        let key_stamp = || async {
            sqlx::query_as::<_, (Option<chrono::DateTime<chrono::Utc>>, Option<String>)>(
                "select last_used_at, host(last_used_ip) from access.api_keys where id = $1",
            )
            .bind(&key_id)
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        let sa_stamp = || async {
            sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
                "select last_used_at from access.service_account_credentials \
                 where service_account_id = $1",
            )
            .bind(&sa_id)
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        let key_bearer = format!("Bearer {token}");
        let sa_bearer = format!("Bearer {sa_token}");

        call(
            &router,
            TRAFFIC,
            &[
                ("x-forwarded-for", "203.0.113.7"),
                ("authorization", key_bearer.as_str()),
            ],
        )
        .await;
        call(&router, TRAFFIC, &[("authorization", sa_bearer.as_str())]).await;
        let (key_used, key_ip) = key_stamp().await;
        assert!(key_used.is_some(), "a key's first use is recorded");
        assert_eq!(key_ip.as_deref(), Some("203.0.113.7"));
        assert!(
            sa_stamp().await.is_some(),
            "a service account's first use is recorded"
        );

        // Two minutes ago: past the throttle, so the next use must refresh it.
        let two_minutes_ago = chrono::Utc::now() - chrono::Duration::minutes(2);
        sqlx::query("update access.api_keys set last_used_at = $2 where id = $1")
            .bind(&key_id)
            .bind(two_minutes_ago)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "update access.service_account_credentials set last_used_at = $2 \
             where service_account_id = $1",
        )
        .bind(&sa_id)
        .bind(two_minutes_ago)
        .execute(&pool)
        .await
        .unwrap();
        call(
            &router,
            TRAFFIC,
            &[
                ("x-forwarded-for", "203.0.113.8"),
                ("authorization", key_bearer.as_str()),
            ],
        )
        .await;
        call(&router, TRAFFIC, &[("authorization", sa_bearer.as_str())]).await;
        let (key_used, key_ip) = key_stamp().await;
        assert!(
            key_used.unwrap() > two_minutes_ago,
            "a stale key stamp is refreshed"
        );
        assert_eq!(
            key_ip.as_deref(),
            Some("203.0.113.8"),
            "with the address of the refreshing use"
        );
        assert!(
            sa_stamp().await.unwrap() > two_minutes_ago,
            "a stale service-account stamp is refreshed"
        );
    }

    /// A delivery to VATUSA's webhook (#605: one division-wide receiver), from `sender`, with a
    /// signature nothing can verify.
    async fn deliver(router: &Router, sender: &str) -> http::StatusCode {
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("/api/v1/webhooks/vatusa")
            .header("x-forwarded-for", sender)
            .header("content-type", "application/json")
            .header("x-mithril-signature", "sha256=00")
            .body(Body::from(r#"{"type":"ping"}"#))
            .unwrap();
        router.clone().oneshot(request).await.unwrap().status()
    }

    /// The exemption covers deliveries the sender isn't at fault for. The handler reads the webhook's
    /// secret from the database before it can verify anything, so a delivery it refuses with a 4xx —
    /// here a 404, nothing registered — is charged to its sender, and once that is spent the sender is
    /// refused before the handler and its database read. Other senders are unaffected.
    #[sqlx::test]
    async fn forged_webhook_deliveries_are_limited_per_sender(pool: PgPool) {
        let router = router(test_state(pool, Default::default()));
        let (vatusa, forger) = ("198.51.100.7", "203.0.113.66");

        // A bucket of two: the third failure spends it, and from then on the forger is refused.
        for attempt in 1..=3 {
            assert_eq!(
                deliver(&router, forger).await,
                http::StatusCode::NOT_FOUND,
                "failure {attempt} reaches the handler"
            );
        }
        for _ in 0..2 {
            assert_eq!(
                deliver(&router, forger).await,
                http::StatusCode::TOO_MANY_REQUESTS,
                "a forger whose failures are spent is refused before the handler"
            );
        }
        assert_eq!(
            deliver(&router, vatusa).await,
            http::StatusCode::NOT_FOUND,
            "another sender is still heard"
        );
    }

    /// #588 review: VATUSA does not retry a delivery, so one the sender isn't at fault for is never
    /// charged — however many arrive — and spends nothing: the same address's first failure is still
    /// heard, and its ordinary anonymous allowance is untouched. A registered webhook OIS can't open
    /// (no usable `OIS_SECRET_KEY` here) answers 503: a server-side refusal, like a genuine delivery
    /// in that the sender did nothing wrong.
    #[sqlx::test]
    async fn deliveries_the_sender_is_not_at_fault_for_are_never_charged(pool: PgPool) {
        sqlx::query(
            "insert into identity.vatusa_webhook (url, secret_ciphertext, key_version) \
             values ('https://example.invalid', '\\x00', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let router = router(test_state(pool.clone(), Default::default()));
        let sender = "198.51.100.7";

        for _ in 0..5 {
            assert_eq!(
                deliver(&router, sender).await,
                http::StatusCode::SERVICE_UNAVAILABLE,
                "never refused for rate, past any allowance"
            );
        }
        sqlx::query("delete from identity.vatusa_webhook")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            deliver(&router, sender).await,
            http::StatusCode::NOT_FOUND,
            "and spent none: the sender's first failure is still heard, not refused"
        );
        let first = call(&router, TRAFFIC, &[("x-forwarded-for", sender)]).await;
        assert_eq!(first.status(), http::StatusCode::OK);
        assert_eq!(
            header(&first, "ratelimit-remaining"),
            "1",
            "nothing was spent from the ordinary allowance"
        );
    }

    /// Health checks, metrics scrapes and the docs must not fail because a caller is over its limit.
    #[tokio::test]
    async fn paths_outside_the_api_are_never_limited() {
        let router = router(AppState::without_db());
        for _ in 0..5 {
            let response = call(&router, "/health", &[]).await;
            assert_ne!(response.status(), http::StatusCode::TOO_MANY_REQUESTS);
            assert!(!response.headers().contains_key("ratelimit-limit"));
        }
    }

    #[test]
    fn whole_seconds_rounds_up() {
        assert_eq!(whole_seconds(Duration::from_millis(1)), 1);
        assert_eq!(whole_seconds(Duration::from_secs(30)), 30);
        assert_eq!(whole_seconds(Duration::ZERO), 0);
    }

    /// #611: an API key for `owner`, by its bearer token, returning its id.
    async fn api_key(pool: &PgPool, owner: &str, token: &str) -> String {
        sqlx::query_scalar(
            "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
             values ($1, $2, 'ois_pat_test', $3) returning id",
        )
        .bind(owner)
        .bind(token)
        .bind(crate::repos::access::sha256_hex(token))
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// A signed-in user holding `permission` nationally, as a session cookie.
    async fn holder_of(pool: &PgPool, permission: &str) -> String {
        let user = seed_user(pool).await;
        crate::scope_test_support::grant(pool, &user, permission, None).await;
        session_cookie(pool, &user).await
    }

    async fn put_limit(
        router: &Router,
        uri: &str,
        cookie: &str,
        per_min: serde_json::Value,
    ) -> u16 {
        let request = http::Request::builder()
            .method(http::Method::PUT)
            .uri(uri)
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"rate_limit_per_min": per_min}).to_string(),
            ))
            .unwrap();
        router
            .clone()
            .oneshot(request)
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    /// How many of `n` calls with `headers` are served before the first 429.
    async fn served(router: &Router, headers: &[(&str, &str)], n: usize) -> usize {
        for i in 0..n {
            if call(router, TRAFFIC, headers).await.status() == http::StatusCode::TOO_MANY_REQUESTS
            {
                return i;
            }
        }
        n
    }

    /// #611 AC1 + AC3: an admin raises one key's limit and only that key gets it — it is served past
    /// the default (2 here) while another key is refused — and clearing it restores the default.
    #[sqlx::test]
    async fn an_overridden_key_is_served_past_the_default_until_cleared(pool: PgPool) {
        let owner = seed_user(&pool).await;
        let partner = api_key(&pool, &owner, "ois_pat_partner").await;
        api_key(&pool, &owner, "ois_pat_everyone").await;
        let admin = holder_of(&pool, "api_keys.key.delete").await;
        // Admin calls go through their own router so they don't spend the session bucket under test.
        let admin_router = router(test_state(pool.clone(), Default::default()));
        let router = router(test_state(pool, Default::default()));
        let uri = format!("/api/v1/admin/api-keys/{partner}/rate-limit");
        let (partner_key, everyone_key) = (
            [("authorization", "Bearer ois_pat_partner")],
            [("authorization", "Bearer ois_pat_everyone")],
        );

        assert_eq!(put_limit(&admin_router, &uri, &admin, 5.into()).await, 200);
        assert_eq!(
            served(&router, &partner_key, 10).await,
            5,
            "the partner gets its override"
        );
        assert_eq!(
            served(&router, &everyone_key, 10).await,
            2,
            "every other key keeps the default"
        );

        assert_eq!(
            put_limit(&admin_router, &uri, &admin, serde_json::Value::Null).await,
            200
        );
        assert_eq!(
            served(&router, &partner_key, 10).await,
            2,
            "cleared, it's back on the default"
        );
    }

    /// #611 AC1, for a service account.
    #[sqlx::test]
    async fn an_overridden_service_account_gets_its_own_limit(pool: PgPool) {
        let account: String = sqlx::query_scalar(
            "insert into access.service_accounts (key, name) values ('vtbfm', 'vTBFM') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.service_account_credentials (service_account_id, secret_hash) \
             values ($1, $2)",
        )
        .bind(&account)
        .bind(crate::repos::access::sha256_hex("ois_sa_vtbfm"))
        .execute(&pool)
        .await
        .unwrap();
        let admin = holder_of(&pool, "service_accounts.update").await;
        let admin_router = router(test_state(pool.clone(), Default::default()));
        let router = router(test_state(pool, Default::default()));
        let uri = format!("/api/v1/admin/service-accounts/{account}/rate-limit");

        assert_eq!(put_limit(&admin_router, &uri, &admin, 4.into()).await, 200);
        assert_eq!(
            served(&router, &[("authorization", "Bearer ois_sa_vtbfm")], 10).await,
            4
        );
    }

    /// A limit must be a positive whole number, the credential must exist, and only an admin may set it.
    #[sqlx::test]
    async fn an_override_is_validated_and_admin_only(pool: PgPool) {
        let owner = seed_user(&pool).await;
        let key = api_key(&pool, &owner, "ois_pat_validate").await;
        let admin = holder_of(&pool, "api_keys.key.delete").await;
        let owner_cookie = holder_of(&pool, "api_keys.key.create").await;
        let router = build_router_with_limits(
            test_state(pool.clone(), Default::default()),
            Arc::new(RateLimits::from_env()),
        );
        let uri = format!("/api/v1/admin/api-keys/{key}/rate-limit");

        assert_eq!(put_limit(&router, &uri, &admin, 0.into()).await, 400);
        assert_eq!(put_limit(&router, &uri, &admin, (-5).into()).await, 400);
        let missing = "/api/v1/admin/api-keys/nope/rate-limit";
        assert_eq!(put_limit(&router, missing, &admin, 5.into()).await, 404);
        assert_eq!(
            put_limit(&router, &uri, &owner_cookie, 500.into()).await,
            401,
            "not an admin"
        );
        let stored: Option<i32> =
            sqlx::query_scalar("select rate_limit_per_min from access.api_keys where id = $1")
                .bind(&key)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, None, "nothing refused was written");
    }

    /// #611 AC2: a key's owner and admins see its request volume and 429 count, summed across flushes
    /// (each replica adds its own).
    #[sqlx::test]
    async fn a_keys_owner_and_admins_see_its_requests_and_refusals(pool: PgPool) {
        let owner = seed_user(&pool).await;
        crate::scope_test_support::grant(&pool, &owner, "api_keys.key.create", None).await;
        let owner_cookie = session_cookie(&pool, &owner).await;
        api_key(&pool, &owner, "ois_pat_counted").await;
        let admin = holder_of(&pool, "api_keys.key.read").await;
        let two = NonZeroU32::new(2).unwrap();
        let limits = Arc::new(RateLimits::new(two, two, two));
        let limited =
            build_router_with_limits(test_state(pool.clone(), Default::default()), limits.clone());
        let viewer = build_router_with_limits(
            test_state(pool.clone(), Default::default()),
            Arc::new(RateLimits::from_env()),
        );
        let key = [("authorization", "Bearer ois_pat_counted")];

        assert_eq!(
            served(&limited, &key, 3).await,
            2,
            "2 served, the 3rd refused"
        );
        crate::repos::credential_usage::add(&pool, &limits.take_usage())
            .await
            .unwrap();
        call(&limited, TRAFFIC, &key).await; // refused again, in a later flush
        crate::repos::credential_usage::add(&pool, &limits.take_usage())
            .await
            .unwrap();

        let expected = serde_json::json!({
            "requests_this_hour": 4, "requests_last_day": 4, "refused_last_day": 2
        });
        for (uri, cookie) in [
            ("/api/v1/api-keys", &owner_cookie),
            ("/api/v1/admin/api-keys", &admin),
        ] {
            let response = call(&viewer, uri, &[("cookie", cookie.as_str())]).await;
            assert_eq!(response.status(), http::StatusCode::OK, "{uri}");
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body[0]["usage"], expected, "{uri}");
        }
    }
}
