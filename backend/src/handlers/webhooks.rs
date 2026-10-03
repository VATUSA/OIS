//! Inbound webhook receivers. Currently just VATUSA's division webhook (the "mithril" v3 outbound
//! webhook), verified by HMAC and used to bring the division pull forward.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::{config::ois_secret_key, feed::vatusa, repos::vatusa as repo, state::AppState};

type HmacSha256 = Hmac<Sha256>;

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
    let secret = match receiver_secret(pool, ois_secret_key()).await {
        Ok(secret) => secret,
        Err(status) => return status,
    };
    if !signature_is_valid(&secret, &headers, &body) {
        return StatusCode::UNAUTHORIZED;
    }

    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
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
}
