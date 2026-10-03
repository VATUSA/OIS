//! Event handlers — read the VATUSA event cache that anchors per-event planning.

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Datelike, Utc, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{
            EventsDebriefCreate, EventsDiscordPublish, EventsPlanRead, EventsPlanUpdate,
            EventsRateUpdate, EventsSupportUpdate, StatsCaptureUpdate,
        },
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed,
    models::{
        AddPackageItemRequest, AirportRateBody, AirportStatBody, CombinedStatBody,
        CreateGroundStopRequest, CreatePackageRequest, CreateTmiRequest, DccRequestBody,
        EventAvailabilityBody, EventBody, EventCaptureBody, EventDebriefBody, EventStatsBody,
        FacilitySupportBody, FcaBody, KeyCountBody, SetFcaAutoRequest, Tier1GenerateResult,
        TmiPackageBody, UpdateDccRequest, UpdateEventCaptureRequest, UpdateEventDebriefRequest,
        UpsertAirportRateRequest, UpsertFacilitySupportRequest, UpsertFcaRequest,
        UpsertProgramRequest,
    },
    repos::{
        access as access_repo, ace as ace_repo, availability as availability_repo,
        events as events_repo, flow as flow_repo, integration as integration_repo, org as org_repo,
        stats as stats_repo, tmu as tmu_repo,
    },
    state::AppState,
};

// --- TMI-package item payloads (the create-shape per kind) ---

#[derive(Debug, Deserialize, Serialize)]
struct ProgramItem {
    icao: String,
    aar: i32,
    #[serde(default)]
    trail: i32,
    #[serde(default)]
    mit: i32,
}

#[derive(Debug, Deserialize, Serialize)]
struct RestrictionItem {
    requesting: String,
    providing: String,
    #[serde(default)]
    restriction: String,
    /// Structured NTML fields — when present the raw `restriction` is derived (encoded) from them and
    /// the decoded English is generated at activation.
    #[serde(default)]
    structured: Option<crate::models::NtmlRestriction>,
    #[serde(default)]
    start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    stop_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct GroundStopItem {
    airport: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    until: Option<String>,
}

/// Validate + normalize a package item's payload for its kind (canonical for storage).
fn normalize_item(kind: &str, payload: Value) -> Result<Value, ApiError> {
    match kind {
        "program" => {
            let mut p: ProgramItem =
                serde_json::from_value(payload).map_err(|_| ApiError::BadRequest)?;
            p.icao = normalize_icao(&p.icao).ok_or(ApiError::BadRequest)?;
            if !(0..=200).contains(&p.aar) {
                return Err(ApiError::BadRequest);
            }
            p.trail = p.trail.max(0);
            p.mit = p.mit.max(0);
            serde_json::to_value(p).map_err(|_| ApiError::Internal)
        }
        "restriction" => {
            let mut r: RestrictionItem =
                serde_json::from_value(payload).map_err(|_| ApiError::BadRequest)?;
            // Uppercased like every other TMI write path: activation stores these as typed, and the
            // facility map is keyed uppercase, so `zdc` resolved to no ARTCC and the TMI reached no
            // one outside national TMU (VATUSA/OIS#405 review).
            r.requesting = r.requesting.trim().to_ascii_uppercase();
            r.providing = r.providing.trim().to_ascii_uppercase();
            // A structured restriction derives its raw line; a raw one uses the typed text.
            if let Some(s) = &r.structured {
                r.restriction = crate::tmi::encode(s);
            }
            r.restriction = r.restriction.trim().to_string();
            if r.requesting.is_empty() || r.providing.is_empty() || r.restriction.is_empty() {
                return Err(ApiError::BadRequest);
            }
            serde_json::to_value(r).map_err(|_| ApiError::Internal)
        }
        "ground_stop" => {
            let mut g: GroundStopItem =
                serde_json::from_value(payload).map_err(|_| ApiError::BadRequest)?;
            g.airport = normalize_icao(&g.airport).ok_or(ApiError::BadRequest)?;
            g.scope = g
                .scope
                .map(|s| s.trim().to_ascii_uppercase())
                .filter(|s| !s.is_empty());
            g.until = g
                .until
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            serde_json::to_value(g).map_err(|_| ApiError::Internal)
        }
        _ => Err(ApiError::BadRequest),
    }
}

const DCC_STATUSES: [&str; 3] = ["not_needed", "requested", "confirmed"];
const SUPPORT_LEVELS: [&str; 3] = ["required", "preferred", "not_required"];
const RATE_PERMISSION: &str = "events.rate.update";
const SUPPORT_PERMISSION: &str = "events.support.update";

pub(crate) fn normalize_facility(raw: &str) -> Option<String> {
    let f = raw.trim().to_ascii_uppercase();
    (!f.is_empty() && f.len() <= 8 && f.chars().all(|c| c.is_ascii_alphanumeric())).then_some(f)
}

pub(crate) fn normalize_icao(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_uppercase();
    (s.len() >= 3 && s.len() <= 4 && s.chars().all(|c| c.is_ascii_alphanumeric())).then_some(s)
}

/// The ARTCC that owns `icao`, from the live facility map (None if unknown).
pub(crate) async fn owning_artcc(state: &AppState, icao: &str) -> Option<String> {
    let map = state.facilities.read().await;
    feed::facilities::artcc_for_airport(&map, icao)
}

fn default_dcc() -> DccRequestBody {
    DccRequestBody {
        status: "not_needed".to_string(),
        notes: String::new(),
        updated_at: None,
        updated_by: None,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/events",
    tag = "events",
    responses((status = 200, body = Vec<EventBody>), (status = 401))
)]
pub async fn list_events(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
) -> Result<Json<Vec<EventBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(events_repo::list_all(pool).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventBody), (status = 401), (status = 404))
)]
pub async fn get_event(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<EventBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    events_repo::get(pool, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// How long the banner fetch gets before it is abandoned. Short: a page is waiting on it.
const BANNER_TIMEOUT: Duration = Duration::from_secs(10);

/// The most banner we will relay. Event banners are a few hundred KB; this only stops a hostile or
/// broken upstream streaming indefinitely into our memory.
const BANNER_MAX_BYTES: usize = 8 * 1024 * 1024;

/// One client for every banner fetch, rather than one per request.
///
/// A `reqwest::Client` owns a connection pool and a TLS config; building one per call throws both
/// away each time. That is tolerable in the feed pollers, which run on a timer — this is a *request*
/// handler, and the events list renders one banner per row, so a page of N events built N clients
/// and reused no connection (#429 review). Same shape as `feed/forecast.rs`.
/// A DNS resolver that refuses to hand back an address inside our own network.
///
/// The filter lives *in* resolution rather than in a check before the fetch, which matters: a check
/// then a separate connect is a check-then-use gap — the name can answer differently the second time
/// (DNS rebinding). Here the addresses the connector is given are the only ones it can dial, and
/// every request through this client goes through it.
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                .collect();
            // All or nothing: a name answering with both a public and a private address is not one
            // we follow at all, rather than one we follow to whichever came first.
            if addrs.is_empty() || addrs.iter().any(|a| !is_public_ip(&a.ip())) {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "banner host resolves to a non-public address",
                ));
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// One client for every banner fetch, rather than one per request.
///
/// A `reqwest::Client` owns a connection pool and a TLS config; building one per call throws both
/// away each time. That is tolerable in the feed pollers, which run on a timer — this is a *request*
/// handler, and the events list renders one banner per row, so a page of N events built N clients
/// and reused no connection (#429 review). Same shape as `feed/forecast.rs`.
static BANNER_CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(BANNER_TIMEOUT)
        // A redirect is how an allowed-looking URL becomes a disallowed one — and how an address
        // filter would be sidestepped after the fact.
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
        .build()
        .expect("banner http client")
});

/// Why a banner could not be relayed. Separate from the response so each guard can be tested.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BannerReject {
    /// The event has no banner, or no such event.
    Missing,
    /// Plain http, or not a URL we will fetch at all.
    NotHttps,
    /// The host answered, but not with a usable image.
    Unusable,
}

/// Whether a banner URL is one we will fetch at all, before any connection is made.
///
/// Only the scheme here — *where* it may go is the resolver's job ([`PublicOnlyResolver`]), because
/// a check made here and a connection made later is a gap the name can be changed inside.
pub(crate) fn check_banner_scheme(url: &str) -> Result<(), BannerReject> {
    // Plain http would let anything on the path swap the image, and a scheme like `file:` has no
    // business reaching a fetcher at all. Judged on the parsed scheme rather than a `starts_with`,
    // which was the same check twice — dropping the prefix test changed no behaviour and no test
    // (#429 review), so the parse is the one that earns its place.
    let parsed = reqwest::Url::parse(url).map_err(|_| BannerReject::NotHttps)?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(BannerReject::NotHttps);
    }
    Ok(())
}

/// Whether an address is one we are willing to fetch from — i.e. out on the internet.
///
/// An allowlist of "public" rather than a blocklist of known-bad: the ranges that must not be
/// reachable from a URL someone else supplies are loopback, link-local (which is where cloud
/// metadata lives), the RFC1918 blocks, carrier-grade NAT, and IPv6's unique-local and mapped-v4
/// forms. Anything unrecognised is refused rather than allowed.
pub(crate) fn is_public_ip(ip: &std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || o[0] == 0
                || o[0] >= 224                       // multicast + reserved
                || (o[0] == 100 && (64..128).contains(&o[1]))) // 100.64/10 CGNAT
        }
        IpAddr::V6(v6) => {
            // A v4 address wearing a v6 hat is still that v4 address.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(&IpAddr::V4(v4));
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00       // fc00::/7 unique-local
                || (seg[0] & 0xffc0) == 0xfe80) // fe80::/10 link-local
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/banner",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses(
        (status = 200, description = "The event's banner image", content_type = "image/*"),
        (status = 401),
        (status = 404, description = "No such event, or it has no banner"),
        (status = 503, description = "The banner's host did not return a usable image")
    )
)]
/// Relays an event's banner image through the API (#429).
///
/// Banners are third-party URLs mirrored from VATUSA, and organisers use whatever host they like —
/// five unrelated ones are in the data already. The bundled desktop app runs under a CSP whose
/// `img-src` cannot name them all without becoming `https:`, so the image is fetched here and served
/// from our own origin instead. The caller turns it into a `blob:` URL, which the policy does allow.
///
/// Guarded, because this makes the backend fetch a URL someone else controls: `https` only, public
/// addresses only, no redirects, a short timeout, a size cap, and an `image/*` response or nothing.
// Where it may go is `PublicOnlyResolver`; what may come back is `relay_banner`. Kept out of the doc
// comment above because utoipa publishes that verbatim into the OpenAPI description, and an API
// consumer has no way to look up a Rust symbol (#429 review).
pub async fn get_event_banner(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;

    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let event = events_repo::get(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let (content_type, bytes) = relay_banner(&event.banner_image_url).await.map_err(|e| {
        match e {
            BannerReject::Missing => ApiError::NotFound,
            // Deliberately the same answer for the rest: the caller learns "no banner", not whether
            // the URL was internal, unreachable, or simply not an image.
            _ => ApiError::ServiceUnavailable,
        }
    })?;

    Ok((
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            // Banners change when an organiser edits the event, which is rare; an hour keeps the
            // page snappy without pinning a stale image for long.
            (
                axum::http::header::CACHE_CONTROL,
                "private, max-age=3600".to_string(),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// Fetch a banner and hand back its content type and bytes, or why not.
///
/// Split from the handler so every guard here is reachable from a test: `RequirePermission` holds a
/// private field, and the guards are the whole safety story of an endpoint that fetches somebody
/// else's URL — they were deletable with the entire suite green (#429 review).
pub(crate) async fn relay_banner(url: &str) -> Result<(String, Vec<u8>), BannerReject> {
    if url.is_empty() {
        return Err(BannerReject::Missing);
    }
    check_banner_scheme(url)?;

    // A private address never resolves through this client, so a URL aimed inside the network fails
    // here as a transport error rather than being fetched (see `PublicOnlyResolver`).
    let res = BANNER_CLIENT
        .get(url)
        .send()
        .await
        .map_err(|_| BannerReject::Unusable)?;
    if !res.status().is_success() {
        return Err(BannerReject::Unusable);
    }

    let content_type = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !content_type.starts_with("image/") {
        return Err(BannerReject::Unusable);
    }

    // Refuse on the declared length before reading anything, then cap while reading anyway: a
    // hostile or broken host can lie about `Content-Length`, or omit it, and `bytes()` would buffer
    // the whole body into memory before any size check could run.
    if res
        .content_length()
        .is_some_and(|n| n > BANNER_MAX_BYTES as u64)
    {
        return Err(BannerReject::Unusable);
    }

    let mut res = res;
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|_| BannerReject::Unusable)? {
        if bytes.len() + chunk.len() > BANNER_MAX_BYTES {
            return Err(BannerReject::Unusable);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((content_type, bytes))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/dcc",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = DccRequestBody), (status = 401))
)]
pub async fn get_event_dcc(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<DccRequestBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(
        events_repo::get_dcc(pool, id)
            .await?
            .unwrap_or_else(default_dcc),
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/dcc",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpdateDccRequest,
    responses((status = 200, body = DccRequestBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_dcc(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateDccRequest>,
) -> Result<Json<DccRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if !DCC_STATUSES.contains(&payload.status.as_str()) {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    let notes = payload.notes.unwrap_or_default();
    events_repo::upsert_dcc(pool, id, &payload.status, notes.trim(), &user.id).await?;
    Ok(Json(
        events_repo::get_dcc(pool, id)
            .await?
            .unwrap_or_else(default_dcc),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/facilities",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<FacilitySupportBody>), (status = 401), (status = 404))
)]
pub async fn list_event_facilities(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<FacilitySupportBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let event = events_repo::get(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Derive the involved facilities from data we already have, unioned with any saved rows:
    //   host ARTCC (event.facility) · owning ARTCCs of configured airports · ACE staffing requests.
    let stored: std::collections::HashMap<String, FacilitySupportBody> =
        events_repo::list_facility_support(pool, id)
            .await?
            .into_iter()
            .map(|r| (r.facility.clone(), r))
            .collect();

    // ARTCC -> its configured airports on this event.
    let mut airports_by_facility: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for r in events_repo::list_airport_rates(pool, id).await? {
        if !r.artcc.is_empty() {
            airports_by_facility
                .entry(r.artcc)
                .or_default()
                .push(r.icao);
        }
    }

    // `has_staffing` now means "this ARTCC has an open ACE request on the event" (the old
    // events.staffing_request source was replaced by event-scoped ACE requests).
    let staffing: std::collections::HashSet<String> = ace_repo::open_request_artccs(pool, id)
        .await?
        .into_iter()
        .collect();

    let host = normalize_facility(&event.facility);

    // Union of every facility id that any signal (or a saved row) surfaced.
    let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    ids.extend(stored.keys().cloned());
    ids.extend(airports_by_facility.keys().cloned());
    ids.extend(staffing.iter().cloned());
    if let Some(h) = host.as_ref() {
        ids.insert(h.clone());
    }

    // Editability is facility-scoped on events.support.update.
    let principal = Principal::optional(current_user.as_ref(), current_api_key.as_ref());
    let scope = match principal.as_ref() {
        Some(p) => Some(p.permission_scope(&state, SUPPORT_PERMISSION).await?),
        None => None,
    };

    let rows: Vec<FacilitySupportBody> = ids
        .into_iter()
        .map(|facility| {
            let is_host = host.as_deref() == Some(facility.as_str());
            let mut airports = airports_by_facility
                .get(&facility)
                .cloned()
                .unwrap_or_default();
            airports.sort();
            let has_staffing = staffing.contains(&facility);
            let editable = scope
                .as_ref()
                .map(|s| s.allows(Some(facility.as_str())))
                .unwrap_or(false);

            match stored.get(&facility) {
                Some(row) => FacilitySupportBody {
                    facility: facility.clone(),
                    level: row.level.clone(),
                    notes: row.notes.clone(),
                    updated_at: row.updated_at,
                    updated_by: row.updated_by.clone(),
                    is_host,
                    airports,
                    has_staffing,
                    stored: true,
                    editable,
                },
                // Derived-only suggestion: default host to required, everything else to preferred.
                None => FacilitySupportBody {
                    facility,
                    level: if is_host { "required" } else { "preferred" }.to_string(),
                    notes: String::new(),
                    updated_at: None,
                    updated_by: None,
                    is_host,
                    airports,
                    has_staffing,
                    stored: false,
                    editable,
                },
            }
        })
        .collect();

    Ok(Json(rows))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/facilities/{facility}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("facility" = String, Path, description = "ARTCC/TRACON id")
    ),
    request_body = UpsertFacilitySupportRequest,
    responses((status = 200, body = FacilitySupportBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn upsert_event_facility(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsSupportUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, facility)): Path<(i64, String)>,
    Json(payload): Json<UpsertFacilitySupportRequest>,
) -> Result<Json<FacilitySupportBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;
    if !SUPPORT_LEVELS.contains(&payload.level.as_str()) {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    // Facility scope: the caller must hold events.support.update nationally or for this facility.
    let scope = principal
        .permission_scope(&state, SUPPORT_PERMISSION)
        .await?;
    if !scope.allows(Some(facility.as_str())) {
        return Err(ApiError::Forbidden);
    }

    let notes = payload.notes.unwrap_or_default();
    events_repo::upsert_facility_support(
        pool,
        id,
        &facility,
        &payload.level,
        notes.trim(),
        principal.user_id().ok_or(ApiError::Forbidden)?,
    )
    .await?;
    let mut row = events_repo::get_facility_support(pool, id, &facility)
        .await?
        .ok_or(ApiError::Internal)?;
    row.stored = true;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/facilities/{facility}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("facility" = String, Path, description = "ARTCC/TRACON id")
    ),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_event_facility(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsSupportUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, facility)): Path<(i64, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;

    // Facility scope: the caller must hold events.support.update nationally or for this facility.
    let scope = principal
        .permission_scope(&state, SUPPORT_PERMISSION)
        .await?;
    if !scope.allows(Some(facility.as_str())) {
        return Err(ApiError::Forbidden);
    }

    if events_repo::delete_facility_support(pool, id, &facility).await? {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Fan out ACE support requests to the host ARTCC's **Tier-1 neighbours** — the auto-request half of
/// FNO planning, part of setting up facility support for the event. Only valid for a Friday (UTC)
/// event (the FNO definition). Idempotent: neighbours that already have an open request on the event
/// are skipped, so re-running never double-posts.
#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/facilities/tier1",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses(
        (status = 200, body = Tier1GenerateResult),
        (status = 400, description = "Event is not a Friday (not an FNO)"),
        (status = 401), (status = 404)
    )
)]
pub async fn generate_tier1(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsSupportUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(event_id): Path<i64>,
) -> Result<Json<Tier1GenerateResult>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let event = events_repo::get(p, event_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // FNO = the event starts on a Friday in UTC.
    if event.start_time.weekday() != Weekday::Fri {
        return Err(ApiError::BadRequest);
    }

    // Known OIS facilities (active) — the filter that drops non-OIS (Canadian/oceanic) neighbours.
    let known: HashSet<String> = org_repo::list_facilities(p)
        .await?
        .into_iter()
        .map(|f| f.id)
        .collect();
    let neighbours = feed::neighbors::tier1(&event.facility, &known);

    // Skip neighbours that already have an open request on this event (idempotent re-runs).
    let existing: HashSet<String> = ace_repo::open_request_artccs(p, event_id)
        .await?
        .into_iter()
        .collect();

    // Resolved per-neighbor (not once for the whole batch): each request is routed to the guild
    // serving the neighbor being asked, not the host (#194).
    let mut channels = std::collections::HashMap::new();
    for n in &neighbours {
        let ch =
            integration_repo::channel_id(p, crate::handlers::ace::ACE_CHANNEL, Some(n)).await?;
        channels.insert(n.clone(), ch);
    }
    let date = event.start_time.format("%a, %b %-d").to_string();

    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    for n in neighbours {
        if existing.contains(&n) {
            skipped.push(n);
            continue;
        }
        let details = format!(
            "Tier-1 support for {}'s Friday Night Operation on {date}. Requesting ACE coverage from {n}.",
            event.facility
        );
        let channel = channels.get(&n).cloned().flatten();
        crate::handlers::ace::create_one(
            &mut tx,
            &event,
            channel.as_deref(),
            &user.id,
            &user.display_name,
            Some(&n),
            None,
            1,
            &details,
        )
        .await?;
        created.push(n);
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    Ok(Json(Tier1GenerateResult { created, skipped }))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/rates",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<AirportRateBody>), (status = 401))
)]
pub async fn list_event_rates(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<AirportRateBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut rates = events_repo::list_airport_rates(pool, id).await?;

    // Mark each row editable per the caller's ARTCC scope for events.rate.update.
    if let Some(principal) = Principal::optional(current_user.as_ref(), current_api_key.as_ref()) {
        let scope = principal.permission_scope(&state, RATE_PERMISSION).await?;
        for r in rates.iter_mut() {
            let artcc = (!r.artcc.is_empty()).then_some(r.artcc.as_str());
            r.editable = scope.allows(artcc);
        }
    }
    Ok(Json(rates))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/rates/{icao}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("icao" = String, Path, description = "Airport ICAO")
    ),
    request_body = UpsertAirportRateRequest,
    responses((status = 200, body = AirportRateBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn upsert_event_rate(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsRateUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, icao)): Path<(i64, String)>,
    Json(payload): Json<UpsertAirportRateRequest>,
) -> Result<Json<AirportRateBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    if !(0..=200).contains(&payload.aar) || !(0..=200).contains(&payload.adr) {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    // Facility scope: the caller must hold events.rate.update nationally or for the
    // airport's owning ARTCC.
    let artcc = owning_artcc(&state, &icao).await;
    let scope = principal.permission_scope(&state, RATE_PERMISSION).await?;
    if !scope.allows(artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }

    let source = match payload.source.as_deref() {
        Some("predicted") => "predicted",
        _ => "override",
    };
    events_repo::upsert_airport_rate(
        pool,
        id,
        &icao,
        payload.aar,
        payload.adr,
        artcc.as_deref().unwrap_or(""),
        payload.config_id.as_deref(),
        source,
        principal.user_id().ok_or(ApiError::Forbidden)?,
    )
    .await?;
    let mut row = events_repo::get_airport_rate(pool, id, &icao)
        .await?
        .ok_or(ApiError::Internal)?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/rates/{icao}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("icao" = String, Path, description = "Airport ICAO")
    ),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_event_rate(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsRateUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, icao)): Path<(i64, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    let existing = events_repo::get_airport_rate(pool, id, &icao)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Scope-check against the ARTCC recorded on the row.
    let artcc = (!existing.artcc.is_empty()).then_some(existing.artcc.as_str());
    let scope = principal.permission_scope(&state, RATE_PERMISSION).await?;
    if !scope.allows(artcc) {
        return Err(ApiError::Forbidden);
    }

    events_repo::delete_airport_rate(pool, id, &icao).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- TMI packages ---

/// Confirm a package exists and belongs to `event_id`; returns its status.
async fn package_status(
    pool: &sqlx::PgPool,
    event_id: i64,
    package_id: &str,
) -> Result<String, ApiError> {
    let (owner_event, status) = events_repo::get_package_owner(pool, package_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if owner_event != event_id {
        return Err(ApiError::NotFound);
    }
    Ok(status)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/packages",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401))
)]
pub async fn list_event_packages(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = CreatePackageRequest,
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 400), (status = 401), (status = 404))
)]
pub async fn create_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<CreatePackageRequest>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    events_repo::create_package(pool, id, name, &user.id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/packages/{package_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404))
)]
pub async fn delete_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, package_id)): Path<(i64, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    package_status(pool, id, &package_id).await?;
    events_repo::delete_package(pool, &package_id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages/{package_id}/items",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    request_body = AddPackageItemRequest,
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 400), (status = 401), (status = 404), (status = 409))
)]
pub async fn add_event_package_item(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, package_id)): Path<(i64, String)>,
    Json(payload): Json<AddPackageItemRequest>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if package_status(pool, id, &package_id).await? != "draft" {
        return Err(ApiError::Conflict); // can't edit an activated package
    }
    let canonical = normalize_item(&payload.kind, payload.payload)?;
    events_repo::add_package_item(pool, &package_id, &payload.kind, &canonical).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/packages/{package_id}/items/{item_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id"),
        ("item_id" = String, Path, description = "Item id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404))
)]
pub async fn delete_event_package_item(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, package_id, item_id)): Path<(i64, String, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    package_status(pool, id, &package_id).await?;
    if !events_repo::delete_package_item(pool, &package_id, &item_id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages/{package_id}/activate",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn activate_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, package_id)): Path<(i64, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if package_status(pool, id, &package_id).await? != "draft" {
        return Err(ApiError::Conflict); // already activated
    }
    activate_package(pool, id, &package_id, &user.id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

/// Materialize a draft package's items into the live TMU tables (programs/restrictions/ground stops),
/// recording each item's `live_ref`, then mark the package activated. Shared by the manual Activate
/// handler and the auto-publish scheduler; `actor` is a real user id (stored as created_by/updated_by).
/// Assumes the package is currently a draft.
pub(crate) async fn activate_package(
    pool: &sqlx::PgPool,
    event_id: i64,
    package_id: &str,
    actor: &str,
) -> Result<(), ApiError> {
    let event = events_repo::get(pool, event_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let items = events_repo::list_package_items(pool, package_id).await?;

    // A published restriction posts to Discord like any other; resolve the channel once (None ⇒ skip).
    // Normalized: `event.facility` is a raw passthrough of the external VATUSA v3 events API field
    // (feed/events.rs) with no case/format guarantee, but discord_config_facilities.artcc_id is
    // stored uppercase and matched exactly — an unnormalized value would silently never match,
    // falling back to the pre-#194 first-created-wins behavior with no error.
    let tmu_channel = integration_repo::channel_id(
        pool,
        crate::handlers::tmu::NTML_CHANNEL,
        normalize_facility(&event.facility).as_deref(),
    )
    .await?;

    // Materialize each draft item into the live TMU tables, recording a `live_ref` so the package
    // can later be deactivated (cancelling exactly what it created).
    for item in &items {
        let payload = item.payload.0.clone();
        let live_ref = match item.kind.as_str() {
            "program" => {
                let p: ProgramItem =
                    serde_json::from_value(payload).map_err(|_| ApiError::Internal)?;
                let req = UpsertProgramRequest {
                    aar: p.aar,
                    trail: p.trail,
                    mit: p.mit,
                    gates: Vec::new(),
                    exclude_wake: Vec::new(),
                    exclude_types: Vec::new(),
                    jets_only: false,
                    // planned programs auto-expire an hour after the event ends
                    active_until: Some(event.end_time),
                };
                tmu_repo::upsert_program(pool, &p.icao, &req, &[], actor).await?;
                // Programs are keyed by ICAO; that's the handle for later cleanup.
                p.icao
            }
            "restriction" => {
                let r: RestrictionItem =
                    serde_json::from_value(payload).map_err(|_| ApiError::Internal)?;
                let req = CreateTmiRequest {
                    requesting: r.requesting,
                    providing: r.providing,
                    restriction: r.restriction,
                    structured: r.structured,
                    start_time: r.start_time,
                    stop_time: r.stop_time,
                };
                // Activation goes live: create then publish so the restriction is active.
                let tmi_id = tmu_repo::create_tmi(pool, &req, actor).await?;
                let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
                let tmi = tmu_repo::publish_tmi(&mut tx, &tmi_id, actor).await?;
                if let (Some(tmi), Some(channel_id)) = (tmi, tmu_channel.clone()) {
                    let job = crate::handlers::tmu::tmi_publish_job(&channel_id, &tmi);
                    integration_repo::enqueue_job(
                        &mut tx,
                        "tmi_publish",
                        &job,
                        Some("tmi"),
                        Some(tmi.id.as_str()),
                    )
                    .await?;
                }
                tx.commit().await.map_err(|_| ApiError::Internal)?;
                tmi_id
            }
            "ground_stop" => {
                let g: GroundStopItem =
                    serde_json::from_value(payload).map_err(|_| ApiError::Internal)?;
                let scope = g.scope.clone().unwrap_or_default();
                let req = CreateGroundStopRequest {
                    airport: g.airport.clone(),
                    scope: g.scope.clone(),
                    until: g.until.clone(),
                };
                let gs_id =
                    tmu_repo::create_ground_stop(pool, &req, &scope, g.until.as_deref(), actor)
                        .await?;
                tmu_repo::publish_ground_stop(pool, &gs_id, actor).await?;
                gs_id
            }
            _ => continue,
        };
        events_repo::set_item_live_ref(pool, &item.id, &live_ref).await?;
    }

    events_repo::mark_package_activated(pool, package_id, actor).await?;
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages/{package_id}/deactivate",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn deactivate_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, package_id)): Path<(i64, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if package_status(pool, id, &package_id).await? != "activated" {
        return Err(ApiError::Conflict); // only an activated package can be deactivated
    }
    deactivate_package(pool, &package_id, &user.id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

/// Cancel exactly the live rows a package created (best-effort — a row already cleared manually or
/// auto-expired just returns false), then archive the package. Shared by the manual Deactivate handler
/// and the auto-archive scheduler. Assumes the package is currently activated.
pub(crate) async fn deactivate_package(
    pool: &sqlx::PgPool,
    package_id: &str,
    actor: &str,
) -> Result<(), ApiError> {
    for item in events_repo::list_package_item_refs(pool, package_id).await? {
        let Some(reference) = item.live_ref.as_deref() else {
            continue;
        };
        match item.kind.as_str() {
            "program" => {
                tmu_repo::delete_program(pool, reference).await?;
            }
            "restriction" => {
                tmu_repo::cancel_tmi(pool, reference).await?;
            }
            "ground_stop" => {
                tmu_repo::cancel_ground_stop(pool, reference).await?;
            }
            _ => {}
        }
    }
    events_repo::mark_package_archived(pool, package_id, actor).await?;
    Ok(())
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/packages/{package_id}/auto",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    request_body = SetFcaAutoRequest,
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404))
)]
pub async fn set_event_package_auto(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, package_id)): Path<(i64, String)>,
    Json(payload): Json<SetFcaAutoRequest>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    // Ownership: the package must belong to this event.
    match events_repo::get_package_owner(pool, &package_id).await? {
        Some((event_id, _)) if event_id == id => {}
        _ => return Err(ApiError::NotFound),
    }
    events_repo::set_package_auto(pool, &package_id, payload.auto_publish).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

// --- per-event stats capture + generated stats ---------------------------------------------------

const CAPTURE_PERMISSION: &str = "stats.capture.update";

/// Build the capture-config body (config defaults + current capture status).
async fn capture_body(
    pool: &sqlx::PgPool,
    event_id: i64,
    can_edit: bool,
) -> Result<EventCaptureBody, ApiError> {
    let cfg = stats_repo::get_event_capture(pool, event_id).await?;
    let cap = stats_repo::latest_capture_for_event(pool, event_id).await?;
    Ok(EventCaptureBody {
        enabled: cfg.as_ref().is_some_and(|c| c.enabled),
        pre_minutes: cfg.as_ref().map_or(30, |c| c.pre_minutes),
        post_minutes: cfg.as_ref().map_or(30, |c| c.post_minutes),
        updated_at: cfg.as_ref().map(|c| c.updated_at),
        updated_by: cfg.and_then(|c| c.updated_by),
        capture_status: cap.as_ref().map(|c| c.status.clone()),
        capture_id: cap.as_ref().map(|c| c.id.clone()),
        capture_start: cap.as_ref().map(|c| c.start_time),
        capture_end: cap.and_then(|c| c.end_time),
        can_edit,
    })
}

// --- Event-specific FCAs (planned in the event manager; stored in flow.fca with an event_id) ---

/// Fetch an FCA and confirm it belongs to `event_id` — 404 otherwise, so one event can't touch
/// another's (or a shared) FCA through these routes.
async fn owned_event_fca(
    pool: &sqlx::PgPool,
    event_id: i64,
    fca_id: &str,
) -> Result<FcaBody, ApiError> {
    let fca = flow_repo::get_fca(pool, fca_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if fca.event_id != Some(event_id) {
        return Err(ApiError::NotFound);
    }
    Ok(fca)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/fcas",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404))
)]
pub async fn list_event_fcas(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/fcas",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpsertFcaRequest,
    responses((status = 200, body = Vec<FcaBody>), (status = 400), (status = 401), (status = 404))
)]
pub async fn create_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.name.trim().is_empty() || payload.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    flow_repo::create_event_fca(pool, id, &payload, &user.id).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/fcas/{fca_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    request_body = UpsertFcaRequest,
    responses((status = 200, body = Vec<FcaBody>), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, fca_id)): Path<(i64, String)>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.name.trim().is_empty() || payload.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    owned_event_fca(pool, id, &fca_id).await?;
    flow_repo::update_fca(pool, &fca_id, &payload, &user.id).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/fcas/{fca_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404))
)]
pub async fn delete_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    owned_event_fca(pool, id, &fca_id).await?;
    flow_repo::delete_fca(pool, &fca_id).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/fcas/{fca_id}/publish",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn publish_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = owned_event_fca(pool, id, &fca_id).await?;
    if fca.event_status.as_deref() != Some("planned") {
        return Err(ApiError::Conflict); // only a planned FCA can be published
    }
    flow_repo::mark_event_fca_published(pool, id, &fca_id).await?;
    state.publish(crate::realtime::topic::FCA); // it's live on every map now
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/fcas/{fca_id}/archive",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn archive_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = owned_event_fca(pool, id, &fca_id).await?;
    if !matches!(fca.event_status.as_deref(), Some("planned" | "published")) {
        return Err(ApiError::Conflict); // already archived
    }
    flow_repo::mark_event_fca_archived(pool, id, &fca_id).await?;
    state.publish(crate::realtime::topic::FCA); // archiving a published FCA removes it from live maps
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/fcas/{fca_id}/auto",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    request_body = SetFcaAutoRequest,
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404))
)]
pub async fn set_event_fca_auto(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
    Json(payload): Json<SetFcaAutoRequest>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    owned_event_fca(pool, id, &fca_id).await?;
    flow_repo::set_event_fca_auto(pool, id, &fca_id, payload.auto_publish).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/capture",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventCaptureBody), (status = 401), (status = 404))
)]
pub async fn get_event_capture(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<i64>,
) -> Result<Json<EventCaptureBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let can_edit = match Principal::optional(current_user.as_ref(), current_api_key.as_ref()) {
        Some(principal) => !principal
            .permission_scope(&state, CAPTURE_PERMISSION)
            .await?
            .is_empty(),
        None => false,
    };
    Ok(Json(capture_body(pool, id, can_edit).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/capture",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpdateEventCaptureRequest,
    responses((status = 200, body = EventCaptureBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_capture(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsCaptureUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateEventCaptureRequest>,
) -> Result<Json<EventCaptureBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let pre = payload.pre_minutes.unwrap_or(30).clamp(0, 720);
    let post = payload.post_minutes.unwrap_or(30).clamp(0, 720);
    stats_repo::upsert_event_capture(pool, id, payload.enabled, pre, post, &user.id).await?;
    Ok(Json(capture_body(pool, id, true).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/availability",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<EventAvailabilityBody>), (status = 401))
)]
pub async fn get_event_availability(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<EventAvailabilityBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    availability_repo::list_for_event(pool, id).await.map(Json)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/stats",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventStatsBody), (status = 401), (status = 404))
)]
pub async fn get_event_stats(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<EventStatsBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    event_stats(pool, id).await.map(Json)
}

/// The body of [`get_event_stats`], without the extractors.
///
/// Split out so the window it reports can be asserted on. `RequirePermission` holds a private field,
/// so a gated handler cannot be called directly; the crate's answer to that is
/// [`crate::scope_test_support::send`], which drives a real request through the real router — but it
/// returns a `StatusCode` and nothing else, and what AC 4 is about is a *value in the body*. Rather
/// than widen a helper built for authorization boundaries, the logic moved. Note what that leaves
/// uncovered: the extractor on `get_event_stats` and this delegation. If either needs pinning, extend
/// `send` to hand back the body — not a test-only constructor on a permission type (#433 review).
async fn event_stats(pool: &sqlx::PgPool, id: i64) -> Result<EventStatsBody, ApiError> {
    let Some(event) = events_repo::get(pool, id).await? else {
        return Err(ApiError::NotFound);
    };

    let empty_combined = || CombinedStatBody {
        arrivals: 0,
        departures: 0,
        movements: 0,
        unique_pilots: 0,
        top_aircraft: Vec::new(),
    };

    let Some(cap) = stats_repo::latest_capture_for_event(pool, id).await? else {
        return Ok(EventStatsBody {
            captured: false,
            status: None,
            window_start: None,
            window_end: None,
            airports: Vec::new(),
            combined: empty_combined(),
        });
    };

    // The event's own window, not the capture's (#433). A capture is padded by `pre/post_minutes`
    // — an hour by default — because replay needs lead-in, and that padding has no business in a
    // statistic. Worse, an *open* capture has no `end_time`, and falling back to `Utc::now()` made
    // the window grow until it was closed, so the same event reported a different number every time
    // it was asked. The capture still gates whether anything was recorded at all.
    let from = event.start_time;
    let to = event.end_time;

    // Featured airports = the event's configured (rated) airports.
    let icaos: Vec<String> = events_repo::list_airport_rates(pool, id)
        .await?
        .into_iter()
        .map(|r| r.icao)
        .collect();

    if icaos.is_empty() {
        return Ok(EventStatsBody {
            captured: true,
            status: Some(cap.status),
            window_start: Some(from),
            window_end: Some(to),
            airports: Vec::new(),
            combined: empty_combined(),
        });
    }

    let key_count = |k: stats_repo::KeyCount| KeyCountBody {
        key: k.key,
        count: k.count,
    };

    // Per-airport breakdown + top aircraft (grouped by ICAO).
    //
    // Prefer the copy frozen when the capture closed (#433). Movements come from `stats.flight_leg`,
    // which is pruned at 30 days, so a past event recomputed from legs would report zero once they
    // aged out. The snapshot also carries the window it was taken over, and that is what gets
    // reported: an event rescheduled after its capture closed keeps the counts it earned, and
    // labelling them with the new times would describe them as something they are not.
    //
    // Only a **saved** capture's snapshot, though. A rescheduled event opens a second one, and while
    // that is recording the event is live again — serving the frozen copy then reported the previous
    // run's counts under the previous run's window, with `status: open` beside them, until the new
    // capture closed (#433 review).
    let snapshot = match cap.status.as_str() {
        "saved" => stats_repo::event_movements_snapshot(pool, id).await?,
        _ => None,
    };
    let (breakdown, from, to) = match snapshot {
        Some(snap) => (snap.rows, snap.window_start, snap.window_end),
        None => (
            stats_repo::event_airport_breakdown(pool, &icaos, from, to).await?,
            from,
            to,
        ),
    };
    let mut top_by_icao: std::collections::HashMap<String, Vec<KeyCountBody>> =
        std::collections::HashMap::new();
    for r in stats_repo::event_airport_top_aircraft(pool, &icaos, from, to, 4).await? {
        top_by_icao.entry(r.icao).or_default().push(KeyCountBody {
            key: r.key,
            count: r.count,
        });
    }

    let airports: Vec<AirportStatBody> = breakdown
        .into_iter()
        .map(|b| AirportStatBody {
            movements: b.arrivals + b.departures,
            top_aircraft: top_by_icao.remove(&b.icao).unwrap_or_default(),
            icao: b.icao,
            arrivals: b.arrivals,
            departures: b.departures,
            unique_pilots: b.unique_pilots,
        })
        .collect();

    // Combined: movements sum across airports; pilots deduped; top aircraft across all.
    let arrivals: i64 = airports.iter().map(|a| a.arrivals).sum();
    let departures: i64 = airports.iter().map(|a| a.departures).sum();
    let combined = CombinedStatBody {
        arrivals,
        departures,
        movements: arrivals + departures,
        unique_pilots: stats_repo::event_combined_unique_pilots(pool, &icaos, from, to).await?,
        top_aircraft: stats_repo::event_combined_top_aircraft(pool, &icaos, from, to)
            .await?
            .into_iter()
            .map(key_count)
            .collect(),
    };

    Ok(EventStatsBody {
        captured: true,
        status: Some(cap.status),
        window_start: Some(from),
        window_end: Some(to),
        airports,
        combined,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/debrief",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventDebriefBody), (status = 401))
)]
pub async fn get_event_debrief(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
) -> Result<Json<EventDebriefBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let editable = match current_user.as_ref() {
        Some(u) => access_repo::fetch_user_permission_names(pool, &u.id)
            .await?
            .iter()
            .any(|p| p == "events.debrief.create"),
        None => false,
    };
    let (notes, updated_by, updated_at) = match events_repo::get_debrief(pool, id).await? {
        Some((n, by, at)) => (n, by, Some(at)),
        None => (String::new(), None, None),
    };
    Ok(Json(EventDebriefBody {
        notes,
        updated_by,
        updated_at,
        editable,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/debrief",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpdateEventDebriefRequest,
    responses((status = 200, body = EventDebriefBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_debrief(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsDebriefCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateEventDebriefRequest>,
) -> Result<Json<EventDebriefBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.notes.len() > 20_000 {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    events_repo::upsert_debrief(pool, id, &payload.notes, &user.id).await?;
    let (notes, updated_by, updated_at) = match events_repo::get_debrief(pool, id).await? {
        Some((n, by, at)) => (n, by, Some(at)),
        None => (payload.notes, None, None),
    };
    Ok(Json(EventDebriefBody {
        notes,
        updated_by,
        updated_at,
        editable: true,
    }))
}

/// Fallback channel (a config logical name) when the host has no region / no region channel is mapped.
const EVENTS_CHANNEL: &str = "events";

/// Host ARTCC → DCC region id (from the VATUSA portal's event-region grouping, trimmed to the real
/// facility set). The region id is the suffix of the `region-{id}` channel logical name.
fn dcc_region(facility: &str) -> Option<&'static str> {
    match facility.to_ascii_uppercase().as_str() {
        "ZBW" | "ZDC" | "ZNY" | "ZOB" => Some("northeast"),
        "ZID" | "ZJX" | "ZMA" | "ZTL" => Some("southeast"),
        "ZAB" | "ZFW" | "ZHU" | "ZME" => Some("southcentral"),
        "ZAU" | "ZDV" | "ZKC" | "ZMP" => Some("midwest"),
        "ZAN" | "HCF" | "ZLA" | "ZLC" | "ZOA" | "ZSE" => Some("west"),
        _ => None,
    }
}

#[utoipa::path(
    post, path = "/api/v1/events/{id}/discord/publish", tag = "events",
    params(("id" = i64, Path)),
    responses(
        (status = 202, description = "Thread creation enqueued"),
        (status = 400, description = "No Discord channel configured for this region"),
        (status = 401), (status = 404),
        (status = 409, description = "Already posted for this event")
    )
)]
pub async fn publish_event_discord(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsDiscordPublish>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let event = events_repo::get(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let subject_id = id.to_string();
    // Don't create a second thread if one was already posted (the bot acked a create for this event).
    if integration_repo::succeeded_job_result(pool, "event", &subject_id, "event_thread_create")
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict);
    }

    // Route by the host's DCC region; fall back to the generic `events` channel if unmapped/unconfigured.
    // Both calls are scoped by the host facility (normalized — see activate_package's comment on
    // why raw event.facility can't be trusted as-is) so the guild serving it wins a same-named
    // collision with another guild (#194).
    let facility = normalize_facility(&event.facility);
    let mut channel = None;
    if let Some(region) = dcc_region(&event.facility) {
        channel =
            integration_repo::channel_id(pool, &format!("region-{region}"), facility.as_deref())
                .await?;
    }
    if channel.is_none() {
        channel = integration_repo::channel_id(pool, EVENTS_CHANNEL, facility.as_deref()).await?;
    }
    let channel = channel.ok_or(ApiError::BadRequest)?;

    // Involved facilities = Required/Preferred support rows; each pings the facility's EC(s) — the OIS
    // users holding the `EC` role scoped to that ARTCC (set in Access Control), by their linked Discord.
    let mut facilities = Vec::new();
    for f in events_repo::list_facility_support(pool, id)
        .await?
        .into_iter()
        .filter(|f| matches!(f.level.as_str(), "required" | "preferred"))
    {
        let ec_user_ids = integration_repo::ec_discord_ids(pool, &f.facility).await?;
        facilities.push(serde_json::json!({ "id": f.facility, "ec_user_ids": ec_user_ids }));
    }
    // Unscoped: these read as facility-generic staff roles pinged on every event thread, not roles
    // that vary per facility (#194).
    let ntmo_role_id = integration_repo::role_id(pool, "ntmo", None).await?;
    let dcc_trainee_role_id = integration_repo::role_id(pool, "dcc-trainee", None).await?;
    let thread_template = integration_repo::get_event_thread_template(pool).await?;

    let thread_name = format!("{} {}", event.start_time.format("%Y%m%d"), event.title);
    let date_line = format!(
        "{} · {}z",
        event.start_time.format("%a, %b %-d"),
        event.start_time.format("%H%M")
    );

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let payload = serde_json::json!({
        "channel_id": channel,
        "thread_name": thread_name,
        "event_title": event.title,
        "date_line": date_line,
        "facilities": facilities,
        "ntmo_role_id": ntmo_role_id,
        "dcc_trainee_role_id": dcc_trainee_role_id,
        "event_id": id,
        "thread_template": thread_template,
    });
    integration_repo::enqueue_job(
        &mut tx,
        "event_thread_create",
        &payload,
        Some("event"),
        Some(&subject_id),
    )
    .await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod stats_window_tests {
    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::PgPool;

    use super::*;

    const EVENT: i64 = 700;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, hour, minute, 0).unwrap()
    }

    /// The event runs 12:00–14:00. Its capture is padded by 30 minutes either side, as
    /// `pre/post_minutes` default — so 11:30–14:30 is what the old code reported over.
    async fn seed(pool: &PgPool, capture_end: Option<DateTime<Utc>>) {
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) \
             values ($1, 'Test', $2, $3)",
        )
        .bind(EVENT)
        .bind(at(12, 0))
        .bind(at(14, 0))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("insert into events.airport_rate (event_id, icao) values ($1, 'KJFK')")
            .bind(EVENT)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "insert into stats.capture (id, event_id, start_time, end_time, status) \
             values ('cap-test', $1, $2, $3, $4)",
        )
        .bind(EVENT)
        .bind(at(11, 30))
        .bind(capture_end)
        .bind(if capture_end.is_some() {
            "saved"
        } else {
            "open"
        })
        .execute(pool)
        .await
        .unwrap();
    }

    async fn leg(pool: &PgPool, kind: &str, cid: i32, end: DateTime<Utc>) {
        sqlx::query(
            "insert into stats.flight_leg \
             (kind, airport, callsign, cid, start_time, end_time, duration_sec) \
             values ($1, 'KJFK', $2, $3, $4, $5, 600)",
        )
        .bind(kind)
        .bind(format!("TEST{cid}"))
        .bind(cid)
        .bind(end - chrono::Duration::minutes(10))
        .bind(end)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn stats(pool: &PgPool) -> EventStatsBody {
        event_stats(pool, EVENT).await.expect("stats")
    }

    /// AC4. The capture is padded by an hour because *replay* needs lead-in; that padding has no
    /// business in a statistic. Reverting this to `cap.start_time` left every test green, because
    /// the repo tests are handed a window rather than choosing one (#433 review).
    #[sqlx::test]
    async fn the_window_is_the_events_own_not_the_captures_padded_one(pool: PgPool) {
        seed(&pool, Some(at(14, 30))).await;
        leg(&pool, "departure", 1, at(11, 45)).await; // inside the padding, before the event
        leg(&pool, "departure", 2, at(13, 0)).await; // inside the event
        leg(&pool, "arrival", 3, at(14, 15)).await; // inside the padding, after the event

        let body = stats(&pool).await;

        assert_eq!(body.window_start, Some(at(12, 0)));
        assert_eq!(body.window_end, Some(at(14, 0)));
        assert_eq!(
            body.combined.movements, 1,
            "only the movement inside the event window counts"
        );
    }

    /// AC4, the other half. An open capture has no `end_time`, and falling back to `Utc::now()` made
    /// the window grow until it was closed — the same event answered differently every time it was
    /// asked, and swept up everything that had happened since.
    #[sqlx::test]
    async fn an_open_capture_does_not_stretch_the_window_to_now(pool: PgPool) {
        seed(&pool, None).await;
        leg(&pool, "departure", 4, at(13, 0)).await;
        // Long after the event, and long before "now" — `Utc::now()` is years past 2026-06-01 only
        // if the clock says so, so pin the far side with a leg the old code would have swept in.
        leg(&pool, "departure", 5, at(20, 0)).await;

        let body = stats(&pool).await;

        assert_eq!(body.window_end, Some(at(14, 0)), "not now()");
        assert_eq!(body.combined.movements, 1);
    }

    /// A frozen copy is only preferred while the latest capture is `saved`. Serving it during a
    /// *second, open* capture reported the previous run's counts under the previous run's window while
    /// `status` said `open` beside them, and it stayed that way until the new capture closed
    /// (#433 review).
    #[sqlx::test]
    async fn a_live_recapture_computes_instead_of_serving_the_frozen_copy(pool: PgPool) {
        seed(&pool, None).await; // an `open` capture — the event is being recorded again
        stats_repo::snapshot_event_movements(
            &pool,
            EVENT,
            at(6, 0),
            at(8, 0),
            &[stats_repo::AirportBreakdown {
                icao: "KJFK".into(),
                arrivals: 40,
                departures: 40,
                unique_pilots: 9,
            }],
        )
        .await
        .unwrap();
        leg(&pool, "departure", 6, at(13, 0)).await;

        let body = stats(&pool).await;

        assert_eq!(
            body.combined.movements, 1,
            "the live count, not the 80 frozen from the previous run"
        );
        assert_eq!(body.window_start, Some(at(12, 0)), "and the live window");
        assert_eq!(body.window_end, Some(at(14, 0)));
    }

    /// The frozen copy is served with the window it was taken over, so an event rescheduled after
    /// its capture closed keeps counts that still describe themselves correctly (#433 review).
    #[sqlx::test]
    async fn a_frozen_breakdown_is_served_with_its_own_window(pool: PgPool) {
        seed(&pool, Some(at(14, 30))).await;
        stats_repo::snapshot_event_movements(
            &pool,
            EVENT,
            at(12, 0),
            at(14, 0),
            &[stats_repo::AirportBreakdown {
                icao: "KJFK".into(),
                arrivals: 7,
                departures: 5,
                unique_pilots: 9,
            }],
        )
        .await
        .unwrap();
        // Rescheduled afterwards: the counts are still the ones it earned over the old window.
        sqlx::query("update events.event set start_time = $1, end_time = $2 where id = $3")
            .bind(at(18, 0))
            .bind(at(20, 0))
            .bind(EVENT)
            .execute(&pool)
            .await
            .unwrap();

        let body = stats(&pool).await;

        assert_eq!(
            body.combined.movements, 12,
            "the frozen counts, not a recount"
        );
        assert_eq!(body.window_start, Some(at(12, 0)));
        assert_eq!(body.window_end, Some(at(14, 0)));
    }
}

#[cfg(test)]
mod banner_tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    /// The addresses a banner URL must never reach. Every one of these is somewhere inside a
    /// deployment — loopback, the cloud metadata service, the RFC1918 blocks, CGNAT — and the URL is
    /// chosen by whoever created the VATUSA event, not by us (#429 review).
    #[test]
    fn no_address_inside_our_own_network_counts_as_public() {
        for ip in [
            "127.0.0.1",
            "0.0.0.0",
            "169.254.169.254", // the cloud metadata service
            "10.0.0.5",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1", // carrier-grade NAT
            "224.0.0.1",  // multicast
        ] {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(!is_public_ip(&addr), "{ip} must not be treated as public");
        }
        for ip in [
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:10.0.0.5",
            "::ffff:127.0.0.1",
        ] {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(!is_public_ip(&addr), "{ip} must not be treated as public");
        }
    }

    /// …and the guard has to still allow the actual internet, or banners stop working entirely.
    #[test]
    fn ordinary_public_addresses_are_allowed() {
        for ip in ["1.1.1.1", "104.16.0.1", "8.8.8.8"] {
            assert!(is_public_ip(&ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
        assert!(is_public_ip(&IpAddr::V6(
            "2606:4700::1111".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_public_ip(&IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));
    }

    /// A v4 address wrapped in v6 is still that v4 address; missing this is a standard bypass.
    #[test]
    fn a_v4_mapped_private_address_is_not_laundered_by_the_v6_form() {
        assert!(!is_public_ip(
            &"::ffff:192.168.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_public_ip(&"::ffff:8.8.8.8".parse::<IpAddr>().unwrap()));
    }

    /// Plain http would let anything on the path swap the image, and a non-https scheme is how a
    /// `file:` or `gopher:` URL would otherwise reach the fetcher.
    #[test]
    fn only_https_urls_are_fetched() {
        assert_eq!(
            check_banner_scheme("http://example.com/a.png"),
            Err(BannerReject::NotHttps)
        );
        assert_eq!(
            check_banner_scheme("file:///etc/passwd"),
            Err(BannerReject::NotHttps)
        );
        assert_eq!(
            check_banner_scheme("not a url"),
            Err(BannerReject::NotHttps)
        );
        assert_eq!(check_banner_scheme("https://example.com/a.png"), Ok(()));
    }

    /// An event with no banner is a 404, not an attempt to fetch the empty string.
    #[tokio::test]
    async fn an_empty_banner_url_is_missing_rather_than_fetched() {
        assert_eq!(relay_banner("").await.unwrap_err(), BannerReject::Missing);
    }

    /// End to end through the real client: a URL aimed at loopback must not be fetched. Binding a
    /// listener proves the point either way — if the resolver let it through, this would connect.
    #[tokio::test]
    async fn a_url_aimed_inside_the_network_is_never_connected_to() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connected = Arc::new(AtomicBool::new(false));
        let flag = connected.clone();
        std::thread::spawn(move || {
            listener
                .set_nonblocking(false)
                .and_then(|()| listener.accept())
                .map(|_| flag.store(true, Ordering::SeqCst))
                .ok();
        });

        let err = relay_banner(&format!("https://localhost:{port}/banner.png"))
            .await
            .unwrap_err();

        assert_eq!(err, BannerReject::Unusable, "refused, not fetched");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !connected.load(Ordering::SeqCst),
            "the backend opened a connection to a loopback address"
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::normalize_item;

    /// Event restriction items become live TMIs on activation, stored as typed. The facility map is
    /// keyed uppercase, so a lowercase facility resolved to no ARTCC and the TMI alerted no one but
    /// national TMU (VATUSA/OIS#405 review).
    #[test]
    fn a_restriction_items_facilities_are_uppercased() {
        let item = normalize_item(
            "restriction",
            json!({"requesting": " zdc ", "providing": "zny", "restriction": "20 MIT"}),
        )
        .unwrap();
        assert_eq!(item["requesting"], "ZDC");
        assert_eq!(item["providing"], "ZNY");
    }
}
