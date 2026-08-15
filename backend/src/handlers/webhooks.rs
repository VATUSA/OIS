//! Inbound webhook receivers. Currently just VATUSA roster-change deliveries (the "mithril"
//! v3 outbound webhook), which we verify by HMAC and use to refresh affected members.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::{feed::vatusa, repos::vatusa as repo, state::AppState};

type HmacSha256 = Hmac<Sha256>;

/// `POST /api/v1/webhooks/vatusa/{facility}` — a VATUSA roster-change delivery. Public (no
/// session); authenticated instead by the `X-Mithril-Signature` HMAC over the raw body, keyed
/// with the per-facility secret we stored at registration. On a verified `roster_change` we
/// refresh any affected member we have, then ack — delivery is one-shot with no retries.
pub async fn vatusa_webhook(
    State(state): State<AppState>,
    Path(facility): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let Some(pool) = state.db.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let facility = facility.to_ascii_uppercase();

    let secret = match repo::webhook_secret(pool, &facility).await {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND, // no webhook registered for this facility
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    // Verify `X-Mithril-Signature: sha256=<hex HMAC-SHA256(body, secret)>` (constant-time).
    let Some(sig) = headers
        .get("X-Mithril-Signature")
        .and_then(|v| v.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED;
    };
    let Ok(sig_bytes) = hex::decode(sig.strip_prefix("sha256=").unwrap_or(sig)) else {
        return StatusCode::UNAUTHORIZED;
    };
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key len");
    mac.update(&body);
    if mac.verify_slice(&sig_bytes).is_err() {
        return StatusCode::UNAUTHORIZED;
    }

    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if payload.get("type").and_then(|v| v.as_str()) == Some("roster_change")
        && let Some(data) = payload.get("data")
    {
        let cids: Vec<i64> = vatusa::changed_cids(data).into_iter().collect();
        if let Ok(known) = repo::known_cids(pool, &cids).await {
            for cid in known {
                vatusa::spawn_member_sync(pool.clone(), cid);
            }
        }
    }
    // Ack anything we verified, including event types we don't recognize (per the contract).
    StatusCode::OK
}
