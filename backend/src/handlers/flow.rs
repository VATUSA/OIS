//! Flow handlers — FCA CRUD + a lightweight live-traffic feed for the FCA map.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{
            FlowFcaDelete, FlowFcaRead, FlowFcaUpdate, FlowRouteDelete, FlowRouteUpdate,
        },
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::{
        airports::AirportDb, airspace::Boundaries, fca, nav::NavData, trajectory,
        vatsim::FlightPlan, vatsim::VatsimData, winds::Winds,
    },
    jobs,
    models::{
        AircraftRoute, DataStatus, FcaBody, FcaFlight, ReleaseRequest, ReorderRequest, RouteBody,
        RouteWaypoint, TrafficAircraft, UpsertFcaRequest, UpsertRouteRequest,
    },
    repos::flow as flow_repo,
    state::AppState,
};

/// Frozen releases keyed by callsign: (cta_ms, edct_ms).
type ReleaseMap = HashMap<String, (i64, i64)>;

/// Clone the current feed snapshot + airport database under a brief read lock, so callers
/// can do heavy per-request CPU (route resolution / metering) without holding the feed
/// lock — which would otherwise stall the 15s poller's writes and other readers.
async fn feed_view(state: &AppState) -> (Option<Arc<crate::feed::Snapshot>>, Arc<AirportDb>) {
    let guard = state.feed.read().await;
    (guard.snapshot.clone(), guard.airports.clone())
}

/// Airport-code match, tolerant of a leading `K` (KJFK ~ JFK).
fn airport_match(filter: &str, code: &str) -> bool {
    let (f, c) = (filter.to_ascii_uppercase(), code.to_ascii_uppercase());
    f == c || c.strip_prefix('K') == Some(f.as_str()) || f.strip_prefix('K') == Some(c.as_str())
}

/// Whether the filed route mentions `fix` as a token (revision digit tolerant).
fn route_has_fix(route: &str, fix: &str) -> bool {
    let fix = fix.to_ascii_uppercase();
    route.split_whitespace().any(|tok| {
        let t = tok.split('/').next().unwrap_or("").to_ascii_uppercase();
        t == fix || t.trim_end_matches(|c: char| c.is_ascii_digit()) == fix
    })
}

/// Membership filters (dest / origin / fix / altitude). Altitude is checked only for
/// airborne aircraft (current alt); scope (ARTCC polygon) is not yet enforced.
/// Whether the FCA's crossing point falls within its scoped ARTCCs. An empty scope (or no
/// boundary data) means no restriction.
fn passes_scope(fca: &FcaBody, airspace: &Boundaries, lat: f64, lon: f64) -> bool {
    if fca.scope.is_empty() || airspace.is_empty() {
        return true;
    }
    fca.scope.iter().any(|z| airspace.contains(z, lat, lon))
}

fn passes_filters(fca: &FcaBody, fp: &FlightPlan, alt: i64, airborne: bool) -> bool {
    if !fca.dests.is_empty() && !fca.dests.iter().any(|d| airport_match(d, &fp.arrival)) {
        return false;
    }
    if !fca.origins.is_empty() && !fca.origins.iter().any(|o| airport_match(o, &fp.departure)) {
        return false;
    }
    if !fca.fixes.is_empty() && !fca.fixes.iter().any(|f| route_has_fix(&fp.route, f)) {
        return false;
    }
    if airborne {
        if let Some(min) = fca.min_fl {
            if alt < min as i64 * 100 {
                return false;
            }
        }
        if let Some(max) = fca.max_fl {
            if alt > max as i64 * 100 {
                return false;
            }
        }
    }
    true
}

/// Taxi + spool-up allowance added to a ground aircraft's flight time (the profile model
/// covers the climb itself).
const GROUND_TAXI_SEC: f64 = 8.0 * 60.0;

/// ETA to the FCA crossing via the shared climb-profile + winds model. Airborne aircraft
/// start from their current altitude; ground aircraft climb from the surface and carry a
/// taxi allowance.
fn eta_to_crossing(
    airborne: bool,
    along_nm: f64,
    cur_alt_ft: f64,
    cruise_alt_ft: f64,
    tas: f64,
    headwind: Option<f64>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let from_alt = if airborne { cur_alt_ft } else { 0.0 };
    let mut sec = trajectory::profile_transit_sec(along_nm, from_alt, cruise_alt_ft, tas, headwind);
    if !airborne {
        sec += GROUND_TAXI_SEC;
    }
    now + Duration::seconds(sec as i64)
}

fn validate_fca(req: &UpsertFcaRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    if let Some(m) = &req.mode {
        if m != "rate" && m != "mit" {
            return Err(ApiError::BadRequest);
        }
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/fcas",
    tag = "flow",
    responses((status = 200, body = Vec<FcaBody>), (status = 401))
)]
pub async fn list_fcas(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(flow_repo::list_fcas(pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/flow/fcas",
    tag = "flow",
    request_body = UpsertFcaRequest,
    responses((status = 200, body = FcaBody), (status = 400), (status = 401))
)]
pub async fn create_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<FcaBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_fca(&payload)?;
    let id = flow_repo::create_fca(pool, &payload, &user.id).await?;
    flow_repo::get_fca(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    put,
    path = "/api/v1/flow/fcas/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    request_body = UpsertFcaRequest,
    responses((status = 200, body = FcaBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<FcaBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_fca(&payload)?;
    if !flow_repo::update_fca(pool, &id, &payload, &user.id).await? {
        return Err(ApiError::NotFound);
    }
    flow_repo::get_fca(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    delete,
    path = "/api/v1/flow/fcas/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if flow_repo::delete_fca(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// --- flow map routes (shared filed-route strings, resolved by the nav engine on read) ---

fn validate_route(req: &UpsertRouteRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.route.trim().is_empty() {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

/// Resolve a stored route row to its drawn track via the nav engine.
fn resolve_route_body(nav: &NavData, airports: &AirportDb, row: flow_repo::RouteRow) -> RouteBody {
    let (named, unresolved) = fca::full_route_named(nav, airports, &row.dep, &row.arr, &row.route);
    let points = named.iter().map(|(_, lat, lon)| [*lat, *lon]).collect();
    RouteBody {
        id: row.id,
        name: row.name,
        color: row.color,
        route: row.route,
        dep: row.dep,
        arr: row.arr,
        points,
        unresolved,
        updated_at: row.updated_at,
        updated_by: row.updated_by,
    }
}

/// All shared map routes, each resolved to a track. Visible to anyone who can view the flow
/// map (FlowFcaRead).
#[utoipa::path(
    get,
    path = "/api/v1/flow/routes",
    tag = "flow",
    responses((status = 200, body = Vec<RouteBody>), (status = 401))
)]
pub async fn list_routes(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Result<Json<Vec<RouteBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let rows = flow_repo::list_routes(pool).await?;
    let (_, airports) = feed_view(&state).await;
    let nav_db = state.nav.load_full();
    let nav = nav_db.as_ref();
    let airports = airports.as_ref();
    Ok(Json(
        rows.into_iter()
            .map(|r| resolve_route_body(nav, airports, r))
            .collect(),
    ))
}

async fn route_response(state: &AppState, id: &str) -> Result<Json<RouteBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let row = flow_repo::get_route(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (_, airports) = feed_view(state).await;
    let nav_db = state.nav.load_full();
    Ok(Json(resolve_route_body(
        nav_db.as_ref(),
        airports.as_ref(),
        row,
    )))
}

#[utoipa::path(
    post,
    path = "/api/v1/flow/routes",
    tag = "flow",
    request_body = UpsertRouteRequest,
    responses((status = 200, body = RouteBody), (status = 400), (status = 401))
)]
pub async fn create_route(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRouteUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<UpsertRouteRequest>,
) -> Result<Json<RouteBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_route(&payload)?;
    let id = flow_repo::create_route(pool, &payload, &user.id).await?;
    route_response(&state, &id).await
}

#[utoipa::path(
    put,
    path = "/api/v1/flow/routes/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "Route id")),
    request_body = UpsertRouteRequest,
    responses((status = 200, body = RouteBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_route(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRouteUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<UpsertRouteRequest>,
) -> Result<Json<RouteBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_route(&payload)?;
    if !flow_repo::update_route(pool, &id, &payload, &user.id).await? {
        return Err(ApiError::NotFound);
    }
    route_response(&state, &id).await
}

#[utoipa::path(
    delete,
    path = "/api/v1/flow/routes/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "Route id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_route(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRouteDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if flow_repo::delete_route(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/counts",
    tag = "flow",
    responses((status = 200, body = std::collections::HashMap<String, i64>), (status = 401))
)]
pub async fn fca_counts(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Result<Json<HashMap<String, i64>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fcas = flow_repo::list_fcas(pool).await?;
    let mut counts: HashMap<String, i64> = fcas.iter().map(|f| (f.id.clone(), 0)).collect();
    let active: Vec<&FcaBody> = fcas
        .iter()
        .filter(|f| f.enabled && f.points.0.len() >= 2)
        .collect();
    if active.is_empty() {
        return Ok(Json(counts));
    }

    // Clone the snapshot + airport handles and release the feed lock before the heavy
    // per-aircraft route resolution, so the poller's writes never queue behind this CPU.
    let (snapshot, airports) = feed_view(&state).await;
    let Some(snap) = snapshot else {
        return Ok(Json(counts));
    };
    let airports = airports.as_ref();
    let nav_db = state.nav.load_full();
    let nav = nav_db.as_ref();

    // Resolve each aircraft's route once, then test it against every active FCA.
    let mut tally = |fp: &FlightPlan, lat: f64, lon: f64, hdg: i64, gs: i64, alt: i64| {
        let airborne = gs >= 50;
        let Some(path) = fca::route_path(
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            lat,
            lon,
            hdg,
            gs,
        ) else {
            return;
        };
        for f in &active {
            if !passes_filters(f, fp, alt, airborne) {
                continue;
            }
            if fca::crosses(&path, &f.points.0, airborne, lat, lon, hdg).is_some() {
                *counts.get_mut(&f.id).unwrap() += 1;
            }
        }
    };

    for p in &snap.data.pilots {
        if let Some(fp) = &p.flight_plan {
            tally(
                fp,
                p.latitude,
                p.longitude,
                p.heading,
                p.groundspeed,
                p.altitude,
            );
        }
    }
    for pf in &snap.data.prefiles {
        if let Some(fp) = &pf.flight_plan {
            tally(fp, 0.0, 0.0, 0, 0, 0);
        }
    }
    Ok(Json(counts))
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/aircraft/{callsign}/route",
    tag = "flow",
    params(("callsign" = String, Path, description = "Aircraft callsign")),
    responses((status = 200, body = AircraftRoute), (status = 401), (status = 404))
)]
pub async fn aircraft_route(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Path(callsign): Path<String>,
) -> Result<Json<AircraftRoute>, ApiError> {
    let cs = callsign.to_ascii_uppercase();
    let (snapshot, airports) = feed_view(&state).await;
    let snap = snapshot.ok_or(ApiError::ServiceUnavailable)?;
    let airports = airports.as_ref();
    let nav_db = state.nav.load_full();
    let nav = nav_db.as_ref();

    // Connected pilot: draw the remaining route from its live position.
    if let Some(p) = snap
        .data
        .pilots
        .iter()
        .find(|p| p.callsign.eq_ignore_ascii_case(&cs))
    {
        let fp = p.flight_plan.as_ref().ok_or(ApiError::NotFound)?;
        let (named, unresolved) =
            fca::full_route_named(nav, airports, &fp.departure, &fp.arrival, &fp.route);
        let points = fca::route_path(
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            p.latitude,
            p.longitude,
            p.heading,
            p.groundspeed,
        )
        .unwrap_or_default();
        return Ok(Json(AircraftRoute {
            callsign: cs,
            aircraft_type: fp.aircraft_short.clone(),
            dep: fp.departure.clone(),
            arr: fp.arrival.clone(),
            altitude: p.altitude,
            groundspeed: p.groundspeed,
            route: fp.route.clone(),
            points,
            unresolved,
            waypoints: to_waypoints(named),
            nav_cycle: nav.cycle().to_string(),
        }));
    }

    // Prefile: full filed route (no live position).
    if let Some(pf) = snap
        .data
        .prefiles
        .iter()
        .find(|pf| pf.callsign.eq_ignore_ascii_case(&cs))
    {
        let fp = pf.flight_plan.as_ref().ok_or(ApiError::NotFound)?;
        let (named, unresolved) =
            fca::full_route_named(nav, airports, &fp.departure, &fp.arrival, &fp.route);
        let points = named.iter().map(|(_, lat, lon)| [*lat, *lon]).collect();
        return Ok(Json(AircraftRoute {
            callsign: cs,
            aircraft_type: fp.aircraft_short.clone(),
            dep: fp.departure.clone(),
            arr: fp.arrival.clone(),
            altitude: 0,
            groundspeed: 0,
            route: fp.route.clone(),
            points,
            unresolved,
            waypoints: to_waypoints(named),
            nav_cycle: nav.cycle().to_string(),
        }));
    }

    Err(ApiError::NotFound)
}

fn build_data_status(state: &AppState) -> DataStatus {
    let nav = state.nav.load_full();
    let winds = state.winds.load_full();
    let to_dt = |ms: i64| {
        (ms > 0)
            .then(|| DateTime::from_timestamp_millis(ms))
            .flatten()
    };
    DataStatus {
        nav_cycle: nav.cycle().to_string(),
        nav_source: nav.source().to_string(),
        fixes: nav.fix_count(),
        navaids: nav.navaid_count(),
        airways: nav.airway_count(),
        procedures: nav.procedure_count(),
        nav_refreshed: to_dt(state.nav_refreshed.load(Ordering::Relaxed)),
        winds_stations: winds.station_count(),
        winds_refreshed: to_dt(state.winds_refreshed.load(Ordering::Relaxed)),
    }
}

/// Health of the runtime nav + winds data (cycle, source, counts, last-refresh times).
#[utoipa::path(
    get,
    path = "/api/v1/flow/data-status",
    tag = "flow",
    responses((status = 200, body = DataStatus), (status = 401))
)]
pub async fn data_status(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Json<DataStatus> {
    Json(build_data_status(&state))
}

/// How much of the current live filed traffic the nav engine fully resolves, plus the most
/// common tokens it still can't (surfaces real data-coverage gaps).
#[utoipa::path(
    get,
    path = "/api/v1/flow/route-coverage",
    tag = "flow",
    responses((status = 200, body = crate::feed::coverage::CoverageReport), (status = 401), (status = 503))
)]
pub async fn route_coverage(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Result<Json<crate::feed::coverage::CoverageReport>, ApiError> {
    let (snapshot, airports) = feed_view(&state).await;
    let snap = snapshot.ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(crate::feed::coverage::analyze(
        state.nav.load_full().as_ref(),
        airports.as_ref(),
        &snap.data,
    )))
}

/// Force an immediate nav + winds refresh, then return the updated status. Failures are
/// logged and leave the current data in place.
#[utoipa::path(
    post,
    path = "/api/v1/flow/data-refresh",
    tag = "flow",
    responses((status = 200, body = DataStatus), (status = 401))
)]
pub async fn data_refresh(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
) -> Json<DataStatus> {
    if let Err(e) = jobs::refresh_nav_once(&state.nav, &state.nav_refreshed).await {
        tracing::warn!(error = %e, "manual nav refresh failed");
    }
    let client = reqwest::Client::builder()
        .user_agent("ois-winds/1.0 (+https://vatusa.net)")
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .unwrap_or_default();
    match jobs::refresh_winds_once(&state.feed, &state.winds, &state.winds_refreshed, &client).await
    {
        None => tracing::warn!("manual winds refresh: airport database not loaded yet"),
        Some(n) => tracing::info!(stations = n, "manual winds refresh"),
    }
    Json(build_data_status(&state))
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/traffic",
    tag = "flow",
    responses((status = 200, body = Vec<TrafficAircraft>), (status = 401))
)]
pub async fn list_traffic(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Json<Vec<TrafficAircraft>> {
    let snapshot = state.feed.read().await.snapshot.clone();
    let aircraft = snapshot
        .as_ref()
        .map(|snap| {
            snap.data
                .pilots
                .iter()
                .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
                .map(|p| {
                    let fp = p.flight_plan.as_ref();
                    TrafficAircraft {
                        callsign: p.callsign.clone(),
                        lat: p.latitude,
                        lon: p.longitude,
                        heading: p.heading,
                        gs: p.groundspeed,
                        alt: p.altitude,
                        dep: fp.map(|f| f.departure.clone()).unwrap_or_default(),
                        arr: fp.map(|f| f.arrival.clone()).unwrap_or_default(),
                        actype: fp.map(|f| f.aircraft_short.clone()).unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Json(aircraft)
}

/// Parse an "HHMM"/"HHMMz" Zulu clock into an epoch-ms near `now` (±12h).
fn parse_hhmm_z(s: &str, now: DateTime<Utc>) -> Option<i64> {
    let s = s.trim().trim_end_matches(['z', 'Z']);
    if s.len() != 4 || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h: u32 = s[0..2].parse().ok()?;
    let m: u32 = s[2..4].parse().ok()?;
    let naive = now.date_naive().and_hms_opt(h, m, 0)?;
    let mut t = naive.and_utc().timestamp_millis();
    let now_ms = now.timestamp_millis();
    if t < now_ms - 12 * 3600 * 1000 {
        t += 24 * 3600 * 1000;
    } else if t > now_ms + 12 * 3600 * 1000 {
        t -= 24 * 3600 * 1000;
    }
    Some(t)
}

fn to_waypoints(named: Vec<(String, f64, f64)>) -> Vec<RouteWaypoint> {
    named
        .into_iter()
        .map(|(name, lat, lon)| RouteWaypoint { name, lat, lon })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn fca_flight(
    callsign: &str,
    fp: &FlightPlan,
    status: &str,
    lat: f64,
    lon: f64,
    cross: &fca::FcaCrossing,
    path: Vec<[f64; 2]>,
    eta: Option<DateTime<Utc>>,
    gs: i64,
    alt: i64,
    hdg: i64,
    rel: Option<&(i64, i64)>,
) -> FcaFlight {
    FcaFlight {
        callsign: callsign.to_string(),
        dep: fp.departure.clone(),
        arr: fp.arrival.clone(),
        aircraft_type: fp.aircraft_short.clone(),
        status: status.to_string(),
        lat,
        lon,
        cross_lat: cross.lat,
        cross_lon: cross.lon,
        path,
        distance_nm: cross.along_nm.round() as i64,
        eta,
        cross_time: None,
        delay_min: 0,
        seq: 0,
        edct: rel.and_then(|(_, e)| DateTime::from_timestamp_millis(*e)),
        released: rel.is_some(),
        groundspeed: gs,
        altitude: alt,
        heading: hdg,
    }
}

/// Build the crossing candidates for an FCA from a live snapshot (no metering yet).
#[allow(clippy::too_many_arguments)]
fn build_candidates(
    fca: &FcaBody,
    data: &VatsimData,
    airports: &AirportDb,
    nav: &NavData,
    airspace: &Boundaries,
    winds: &Winds,
    releases: &ReleaseMap,
    now: DateTime<Utc>,
) -> (Vec<FcaFlight>, Vec<fca::MeterInput>) {
    let pts = fca.points.0.clone();
    let mut flights = Vec::new();
    let mut metas = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let airborne = p.groundspeed >= 50;
        if !passes_filters(fca, fp, p.altitude, airborne) {
            continue;
        }
        let Some(path) = fca::route_path(
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            p.latitude,
            p.longitude,
            p.heading,
            p.groundspeed,
        ) else {
            continue;
        };
        let Some(cross) = fca::crosses(&path, &pts, airborne, p.latitude, p.longitude, p.heading)
        else {
            continue;
        };
        if !passes_scope(fca, airspace, cross.lat, cross.lon) {
            continue;
        }
        let cruise = trajectory::parse_alt_ft(&fp.altitude);
        let tas = trajectory::tas_or_default(fp.cruise_tas.parse().unwrap_or(0.0), cruise);
        let headwind = winds.route_headwind(&path, cruise);
        let eta = eta_to_crossing(
            airborne,
            cross.along_nm,
            p.altitude as f64,
            cruise,
            tas,
            headwind,
            now,
        );
        let rel = releases.get(&p.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.timestamp_millis(),
            airborne,
            cross_speed: trajectory::predicted_cross_speed(tas, cruise, headwind),
            frozen_ms: rel.map(|(cta, _)| *cta),
        });
        flights.push(fca_flight(
            &p.callsign,
            fp,
            if airborne { "airborne" } else { "ground" },
            p.latitude,
            p.longitude,
            &cross,
            path,
            Some(eta),
            p.groundspeed,
            p.altitude,
            p.heading,
            rel,
        ));
    }

    for pf in &data.prefiles {
        let Some(fp) = &pf.flight_plan else { continue };
        if !passes_filters(fca, fp, 0, false) {
            continue;
        }
        let Some(path) = fca::route_path(
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            0.0,
            0.0,
            0,
            0,
        ) else {
            continue;
        };
        let Some(cross) = fca::crosses(&path, &pts, false, 0.0, 0.0, 0) else {
            continue;
        };
        if !passes_scope(fca, airspace, cross.lat, cross.lon) {
            continue;
        }
        let (dep_lat, dep_lon) = airports
            .get(&fp.departure.to_ascii_uppercase())
            .copied()
            .unwrap_or((0.0, 0.0));
        let cruise = trajectory::parse_alt_ft(&fp.altitude);
        let tas = trajectory::tas_or_default(fp.cruise_tas.parse().unwrap_or(0.0), cruise);
        let headwind = winds.route_headwind(&path, cruise);
        let eta = eta_to_crossing(false, cross.along_nm, 0.0, cruise, tas, headwind, now);
        let rel = releases.get(&pf.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.timestamp_millis(),
            airborne: false,
            cross_speed: trajectory::predicted_cross_speed(tas, cruise, headwind),
            frozen_ms: rel.map(|(cta, _)| *cta),
        });
        flights.push(fca_flight(
            &pf.callsign,
            fp,
            "proposed",
            dep_lat,
            dep_lon,
            &cross,
            path,
            Some(eta),
            0,
            0,
            0,
            rel,
        ));
    }

    (flights, metas)
}

/// Meter the candidates (auto, or the FCA's manual order) and finalize sequence/delay.
fn finalize(
    fca: &FcaBody,
    mut flights: Vec<FcaFlight>,
    metas: &[fca::MeterInput],
) -> Vec<FcaFlight> {
    let order: Option<Vec<usize>> = (fca.manual_seq && !fca.manual_order.is_empty()).then(|| {
        fca.manual_order
            .iter()
            .filter_map(|cs| flights.iter().position(|f| &f.callsign == cs))
            .collect()
    });
    let metered = fca::meter(metas, &fca.mode, fca.rate, fca.mit, order.as_deref());
    for (f, m) in flights.iter_mut().zip(&metered) {
        f.cross_time = DateTime::from_timestamp_millis(m.sched_ms);
        f.delay_min = (m.delay_sec + 30) / 60;
        f.seq = m.seq;
    }
    flights.sort_by_key(|f| f.seq);
    flights
}

async fn load_releases(pool: &sqlx::PgPool, id: &str) -> Result<ReleaseMap, ApiError> {
    Ok(flow_repo::list_releases(pool, id)
        .await?
        .into_iter()
        .map(|(c, cta, edct)| (c, (cta, edct)))
        .collect())
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/fcas/{id}/traffic",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    responses((status = 200, body = Vec<FcaFlight>), (status = 401), (status = 404))
)]
pub async fn fca_traffic(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Path(id): Path<String>,
) -> Result<Json<Vec<FcaFlight>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = flow_repo::get_fca(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if fca.points.0.len() < 2 {
        return Ok(Json(Vec::new()));
    }
    let releases = load_releases(pool, &id).await?;
    let now = Utc::now();

    // Release the feed lock before the metering CPU (build_candidates resolves every
    // matching route) by cloning the snapshot + airport handles.
    let (snapshot, airports) = feed_view(&state).await;
    let built = snapshot.as_ref().map(|snap| {
        build_candidates(
            &fca,
            &snap.data,
            airports.as_ref(),
            state.nav.load_full().as_ref(),
            state.airspace.as_ref(),
            state.winds.load_full().as_ref(),
            &releases,
            now,
        )
    });
    let Some((flights, metas)) = built else {
        return Ok(Json(Vec::new()));
    };
    Ok(Json(finalize(&fca, flights, &metas)))
}

#[utoipa::path(
    post,
    path = "/api/v1/flow/fcas/{id}/release/{callsign}",
    tag = "flow",
    params(
        ("id" = String, Path, description = "FCA id"),
        ("callsign" = String, Path, description = "Aircraft callsign")
    ),
    request_body = ReleaseRequest,
    responses((status = 200, body = Vec<FcaFlight>), (status = 400), (status = 401), (status = 404))
)]
pub async fn mark_release(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, callsign)): Path<(String, String)>,
    Json(payload): Json<ReleaseRequest>,
) -> Result<Json<Vec<FcaFlight>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = flow_repo::get_fca(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let callsign = callsign.to_ascii_uppercase();
    let now = Utc::now();
    let now_ms = now.timestamp_millis();
    let releases = load_releases(pool, &id).await?;

    // Release the feed lock before the metering CPU (build_candidates resolves every
    // matching route) by cloning the snapshot + airport handles.
    let (snapshot, airports) = feed_view(&state).await;
    let built = snapshot.as_ref().map(|snap| {
        build_candidates(
            &fca,
            &snap.data,
            airports.as_ref(),
            state.nav.load_full().as_ref(),
            state.airspace.as_ref(),
            state.winds.load_full().as_ref(),
            &releases,
            now,
        )
    });
    let Some((mut flights, mut metas)) = built else {
        return Err(ApiError::ServiceUnavailable);
    };
    let ti = flights
        .iter()
        .position(|f| f.callsign == callsign)
        .ok_or(ApiError::NotFound)?; // not currently crossing
    let eta_ms = metas[ti].eta_ms;

    // Committed = every other pinned crossing (airborne ETA or an existing frozen CTA).
    let committed: Vec<i64> = metas
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != ti)
        .filter_map(|(_, m)| {
            if m.airborne {
                Some(m.eta_ms)
            } else {
                m.frozen_ms
            }
        })
        .collect();
    let sep_ms = if fca.mode == "mit" {
        ((fca.mit as f64 / metas[ti].cross_speed.max(60.0)) * 3600.0 * 1000.0) as i64
    } else if fca.rate > 0 {
        (3600.0 / fca.rate as f64 * 1000.0) as i64
    } else {
        0
    };

    let cta = match payload.ready.as_deref().filter(|s| !s.trim().is_empty()) {
        // SET: pin the crossing so wheels-up lands on the requested time.
        Some(ready) => parse_hhmm_z(ready, now).ok_or(ApiError::BadRequest)? + (eta_ms - now_ms),
        // RDY: earliest metered slot.
        None => fca::earliest_slot(eta_ms, &committed, sep_ms),
    };
    let edct = cta - (eta_ms - now_ms);
    flow_repo::upsert_release(pool, &id, &callsign, cta, edct, &user.id).await?;

    // Reflect the new release and re-meter without another snapshot read.
    metas[ti].frozen_ms = Some(cta);
    flights[ti].released = true;
    flights[ti].edct = DateTime::from_timestamp_millis(edct);
    Ok(Json(finalize(&fca, flights, &metas)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/flow/fcas/{id}/release/{callsign}",
    tag = "flow",
    params(
        ("id" = String, Path, description = "FCA id"),
        ("callsign" = String, Path, description = "Aircraft callsign")
    ),
    responses((status = 200, body = Vec<FcaFlight>), (status = 401), (status = 404))
)]
pub async fn clear_release(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<Json<Vec<FcaFlight>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = flow_repo::get_fca(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    flow_repo::delete_release(pool, &id, &callsign.to_ascii_uppercase()).await?;
    let releases = load_releases(pool, &id).await?;
    let now = Utc::now();

    // Release the feed lock before the metering CPU (build_candidates resolves every
    // matching route) by cloning the snapshot + airport handles.
    let (snapshot, airports) = feed_view(&state).await;
    let built = snapshot.as_ref().map(|snap| {
        build_candidates(
            &fca,
            &snap.data,
            airports.as_ref(),
            state.nav.load_full().as_ref(),
            state.airspace.as_ref(),
            state.winds.load_full().as_ref(),
            &releases,
            now,
        )
    });
    let Some((flights, metas)) = built else {
        return Ok(Json(Vec::new()));
    };
    Ok(Json(finalize(&fca, flights, &metas)))
}

#[utoipa::path(
    put,
    path = "/api/v1/flow/fcas/{id}/order",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    request_body = ReorderRequest,
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn reorder_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<ReorderRequest>,
) -> Result<StatusCode, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    // Empty order clears manual mode (back to auto).
    let manual = !payload.order.is_empty();
    if !flow_repo::set_manual_order(pool, &id, &payload.order, manual, &user.id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
