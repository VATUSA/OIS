//! Inbound webhook receivers. Currently just VATUSA's division webhook (the "mithril" v3 outbound
//! webhook), verified by HMAC and used to bring the division pull forward.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

use crate::{config::ois_secret_key, feed::vatusa, repos::vatusa as repo, state::AppState};

type HmacSha256 = Hmac<Sha256>;

/// How long a verified delivery's body is remembered, so the same body can't be acted on twice.
const REPLAY_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Replay protection for verified deliveries (#627). The HMAC proves who sent a body but not when,
/// and v3 signs no timestamp, so a captured delivery would verify forever. Remembering each verified
/// body's SHA-256 until [`REPLAY_WINDOW`] passes without it being seen means a re-sent copy is
/// acknowledged but not acted on.
///
/// In memory and per process: a restart forgets it and each replica guards alone. That is enough
/// because acting on a delivery only brings the idempotent, coalesced division pull forward. The
/// cost is that an identical body VATUSA itself re-sends inside the window is skipped too, which
/// is harmless for the same reason. Only verified bodies are recorded, so an unauthenticated caller
/// can't grow the set.
#[derive(Default)]
pub struct ReplayGuard {
    seen: Mutex<HashMap<[u8; 32], Instant>>,
}

impl ReplayGuard {
    /// Records `body` and reports whether it is new within the window. Forgets expired bodies first,
    /// so the set never holds more than one window's deliveries.
    pub fn first_seen(&self, body: &[u8], now: Instant) -> bool {
        let digest: [u8; 32] = Sha256::digest(body).into();
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|_, at| now.duration_since(*at) < REPLAY_WINDOW);
        seen.insert(digest, now).is_none()
    }
}

/// `POST /api/v1/webhooks/vatusa` — a delivery for the division webhook (#605). Public (no session);
/// authenticated instead by the `X-Mithril-Signature` HMAC over the raw body, keyed with the secret
/// stored (encrypted) at registration.
///
/// A verified `roster_change` **triggers the division pull** rather than fetching the affected members
/// one by one over v2: the pull is one request for everyone, and the job registry coalesces a burst of
/// deliveries into a single run. Anything else verified is acked and ignored — deliveries are one-shot,
/// with no retries, and v3 offers no event selection.
pub async fn vatusa_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let Some(pool) = state.db.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    match receiver_secret(pool, ois_secret_key()).await {
        Ok(secret) => handle_delivery(&state, &secret, &headers, &body),
        Err(status) => status,
    }
}

/// A delivery once the secret is in hand: verify, refuse a replay, then act.
fn handle_delivery(state: &AppState, secret: &str, headers: &HeaderMap, body: &[u8]) -> StatusCode {
    if !signature_is_valid(secret, headers, body) {
        return StatusCode::UNAUTHORIZED;
    }
    if !state.webhook_replays.first_seen(body, Instant::now()) {
        tracing::info!("VATUSA webhook delivery is a replay of one already accepted; ignored");
        return StatusCode::OK;
    }

    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(body) else {
        return StatusCode::BAD_REQUEST;
    };
    if payload.get("type").and_then(|v| v.as_str()) == Some("roster_change") {
        state.jobs.trigger(vatusa::PULL_JOB);
    }
    StatusCode::OK
}

/// The stored secret, decrypted. No webhook registered is a 404; one that can't be read — no
/// `OIS_SECRET_KEY`, or a key that no longer opens it — is a 503 and a warning, never a panic. The
/// daily pull replaces an unreadable webhook.
async fn receiver_secret(pool: &sqlx::PgPool, key: Option<[u8; 32]>) -> Result<String, StatusCode> {
    let row = repo::fetch_webhook(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(key) = key else {
        tracing::warn!("VATUSA webhook delivery refused: OIS_SECRET_KEY is unset or invalid");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    crate::secrets::decrypt(&key, &row.secret_ciphertext).ok_or_else(|| {
        tracing::warn!(
            "VATUSA webhook secret could not be decrypted; it is replaced at the next pull"
        );
        StatusCode::SERVICE_UNAVAILABLE
    })
}

/// `X-Mithril-Signature: sha256=<hex HMAC-SHA256(body, secret)>`, compared in constant time. The key is
/// `secret.as_bytes()`, which is why the stored secret must decrypt byte-identically.
fn signature_is_valid(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let Some(sig) = headers
        .get("X-Mithril-Signature")
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(sig.strip_prefix("sha256=").unwrap_or(sig)) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key len");
    mac.update(body);
    mac.verify_slice(&sig_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    const KEY: [u8; 32] = [3; 32];
    const SECRET: &str = "mithril-secret/with+symbols==";

    async fn store(pool: &PgPool, key: &[u8; 32]) {
        repo::store_webhook(
            pool,
            &repo::StoredWebhook {
                vatusa_id: Some(42),
                url: "https://ois.test/api/v1/webhooks/vatusa".to_string(),
                secret_ciphertext: crate::secrets::encrypt(key, SECRET),
                key_version: crate::secrets::KEY_VERSION,
            },
        )
        .await
        .unwrap();
    }

    fn signed(secret: &str, body: &[u8]) -> HeaderMap {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Mithril-Signature",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
                .parse()
                .unwrap(),
        );
        headers
    }

    /// AC6: the secret is stored encrypted, comes back byte-identical, and a delivery signed with it
    /// still verifies — the HMAC is keyed on its exact bytes.
    #[sqlx::test]
    async fn a_delivery_verifies_against_the_encrypted_secret(pool: PgPool) {
        store(&pool, &KEY).await;
        let stored: Vec<u8> =
            sqlx::query_scalar("select secret_ciphertext from identity.vatusa_webhook")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!stored.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()));

        let secret = receiver_secret(&pool, Some(KEY)).await.unwrap();
        let body = br#"{"type":"roster_change","data":{}}"#;
        assert!(signature_is_valid(&secret, &signed(SECRET, body), body));
        assert!(!signature_is_valid(&secret, &signed("other", body), body));
        assert!(!signature_is_valid(&secret, &HeaderMap::new(), body));
    }

    /// AC6: without a usable key the receiver refuses — it never panics and never verifies against
    /// garbage. No webhook at all is a 404.
    #[sqlx::test]
    async fn a_missing_or_wrong_key_degrades_to_503(pool: PgPool) {
        assert_eq!(
            receiver_secret(&pool, Some(KEY)).await,
            Err(StatusCode::NOT_FOUND)
        );
        store(&pool, &KEY).await;
        assert_eq!(
            receiver_secret(&pool, None).await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(
            receiver_secret(&pool, Some([9; 32])).await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
    }

    /// AC7: one receiver for the division, at a fixed path, reachable unauthenticated through the real
    /// router (the signature is the authentication). The per-facility route is gone.
    #[sqlx::test]
    async fn the_division_receiver_is_routed_and_the_per_facility_one_is_gone(pool: PgPool) {
        let state = crate::scope_test_support::test_state(pool, std::collections::HashMap::new());
        let post = |path: &'static str| {
            let state = state.clone();
            async move {
                crate::scope_test_support::send(
                    &state,
                    http::Method::POST,
                    path,
                    "",
                    Some(serde_json::json!({})),
                )
                .await
            }
        };
        // No webhook stored: the route exists and answers 404 for "nothing registered".
        assert_eq!(
            post("/api/v1/webhooks/vatusa").await,
            http::StatusCode::NOT_FOUND
        );
        let legacy = post("/api/v1/webhooks/vatusa/ZDC").await;
        assert!(
            legacy == http::StatusCode::NOT_FOUND || legacy == http::StatusCode::METHOD_NOT_ALLOWED,
            "{legacy}"
        );
    }

    mod replay {
        use std::time::{Duration, Instant};

        use super::*;
        use crate::handlers::webhooks::ReplayGuard;

        const BODY: &[u8] = br#"{"type":"roster_change","data":{"cid":1}}"#;

        /// Absolute offsets either side of ten minutes, not derived from `REPLAY_WINDOW`, so changing
        /// the constant breaks this test rather than moving with it.
        #[test]
        fn a_body_is_refused_inside_the_window_and_accepted_after_it() {
            let t0 = Instant::now();
            let guard = ReplayGuard::default();
            assert!(guard.first_seen(BODY, t0));
            assert!(!guard.first_seen(BODY, t0 + Duration::from_secs(1)));
            assert!(guard.first_seen(b"another body", t0 + Duration::from_secs(1)));

            let fresh = ReplayGuard::default();
            assert!(fresh.first_seen(BODY, t0));
            assert!(!fresh.first_seen(BODY, t0 + Duration::from_secs(599)));
            // 599 s re-armed it: a replay storm stays suppressed until it goes quiet for the window.
            assert!(!fresh.first_seen(BODY, t0 + Duration::from_secs(1_100)));
            assert!(fresh.first_seen(BODY, t0 + Duration::from_secs(1_701)));
        }

        #[test]
        fn expired_bodies_are_forgotten() {
            let t0 = Instant::now();
            let guard = ReplayGuard::default();
            for i in 0..50u8 {
                guard.first_seen(&[i], t0);
            }
            guard.first_seen(b"later", t0 + Duration::from_secs(601));
            assert_eq!(guard.seen.lock().unwrap().len(), 1);
        }

        /// The pull's trigger handle, so a test can see whether a delivery was acted on.
        fn pull_trigger(state: &AppState) -> std::sync::Arc<tokio::sync::Notify> {
            state
                .jobs
                .register(vatusa::PULL_JOB, "division pull", None, true)
        }

        async fn triggered(notify: &tokio::sync::Notify) -> bool {
            tokio::time::timeout(Duration::from_millis(100), notify.notified())
                .await
                .is_ok()
        }

        /// AC1 + AC2 through the handler's own path: a verified delivery triggers the pull once, the
        /// same delivery re-sent is acked but not acted on, and a new delivery still triggers.
        #[tokio::test]
        async fn a_replayed_delivery_is_acked_but_not_acted_on_again() {
            let state = AppState::without_db();
            let pull = pull_trigger(&state);
            let deliver =
                |body: &[u8]| handle_delivery(&state, SECRET, &signed(SECRET, body), body);

            assert_eq!(deliver(BODY), StatusCode::OK);
            assert!(triggered(&pull).await, "a new delivery triggers the pull");

            assert_eq!(deliver(BODY), StatusCode::OK);
            assert!(!triggered(&pull).await, "the replay was acted on");

            let next = br#"{"type":"roster_change","data":{"cid":2}}"#;
            assert_eq!(deliver(next), StatusCode::OK);
            assert!(
                triggered(&pull).await,
                "a different delivery is not a replay"
            );
        }

        /// The guard only ever sees verified bodies: a forged copy is refused without being recorded,
        /// so it can neither fill the set nor pre-empt the genuine delivery.
        #[tokio::test]
        async fn a_forged_delivery_is_refused_and_not_remembered() {
            let state = AppState::without_db();
            let pull = pull_trigger(&state);

            let forged = handle_delivery(&state, SECRET, &signed("not-the-secret", BODY), BODY);
            assert_eq!(forged, StatusCode::UNAUTHORIZED);
            assert!(!triggered(&pull).await);

            let genuine = handle_delivery(&state, SECRET, &signed(SECRET, BODY), BODY);
            assert_eq!(genuine, StatusCode::OK);
            assert!(
                triggered(&pull).await,
                "the forgery pre-empted the real delivery"
            );
        }
    }
}
