//! Per-caller request rate limiting for the `/api/` surface (#588).
//!
//! Every `/api/` request draws from one bucket chosen by who is calling: an API key or service account
//! by its id, a signed-in user (web cookie or desktop token) by user id, and anyone else by client IP.
//! The caller is already resolved by `auth::middleware::resolve_current_user`, which runs first, so
//! choosing the bucket costs no query. A caller over its allowance gets `429` with `Retry-After`;
//! every limited response carries `RateLimit-Limit` / `RateLimit-Remaining` / `RateLimit-Reset`.
//!
//! The buckets live in this process, so with several backend replicas each enforces its own allowance.

use std::{net::IpAddr, num::NonZeroU32, sync::Arc, time::Duration};

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

/// The three allowances, each a full minute's worth available as a burst.
pub struct RateLimits {
    /// Integrations: API keys and service accounts.
    credential: Limiter,
    /// First-party sessions. Generous — a desktop user with several pop-outs polls ~100–150/min.
    user: Limiter,
    /// Unauthenticated callers, by IP. A signed-out map tab polls ~15/min.
    anonymous: Limiter,
}

impl RateLimits {
    pub fn new(credential: NonZeroU32, user: NonZeroU32, anonymous: NonZeroU32) -> Self {
        Self {
            credential: limiter(credential),
            user: limiter(user),
            anonymous: limiter(anonymous),
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
    fn bucket(&self, request: &Request) -> (&Limiter, Caller) {
        let extensions = request.extensions();
        if let Some(Some(key)) = extensions.get::<Option<CurrentApiKey>>() {
            return (&self.credential, Caller::ApiKey(key.id.clone()));
        }
        if let Some(Some(account)) = extensions.get::<Option<CurrentServiceAccount>>() {
            return (&self.credential, Caller::ServiceAccount(account.id.clone()));
        }
        if let Some(Some(user)) = extensions.get::<Option<CurrentUser>>() {
            return (&self.user, Caller::User(user.id.clone()));
        }
        let ip = crate::repos::audit::client_ip(request.headers()).and_then(|ip| ip.parse().ok());
        (&self.anonymous, Caller::Ip(ip))
    }

    /// Forget callers whose bucket has refilled, so the maps don't grow with every IP ever seen.
    fn retain_recent(&self) {
        self.credential.retain_recent();
        self.user.retain_recent();
        self.anonymous.retain_recent();
    }
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
    let (limiter, caller) = limits.bucket(&request);
    match limiter.check_key(&caller) {
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
}
