//! Flow handlers — FCA CRUD + a lightweight live-traffic feed for the FCA map.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{
            FlowFcaDelete, FlowFcaRead, FlowFcaUpdate, FlowRouteDelete, FlowRouteUpdate, StatsRead,
        },
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::{
        airports::AirportDb, airspace::Boundaries, facilities, fca, flow as feed_flow,
        nav::NavData, predict, runway_db::RunwayDb, taxi_estimate, trajectory, vatsim::FlightPlan,
        vatsim::VatsimData, winds::Winds,
    },
    jobs,
    models::{
        AircraftRoute, AirportGateBody, DataStatus, FcaBody, FcaFlight, FixValidationBody,
        FlightAdvisory, FlightFcaCrossing, FlightGdp, FlightGroundStop, FlightProgram, IdstFlight,
        IdstResponse, ReleaseRequest, ReorderRequest, ResolveRouteRequest, ResolvedRoute,
        RouteBody, RouteWaypoint, TrafficAircraft, UpsertFcaRequest, UpsertRouteRequest,
    },
    repos::{flow as flow_repo, public as public_repo},
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

/// A prefile's stand-in position for route resolution (#213) — it has no live coordinates, so use
/// its departure airport's, matching `feed::flow::ground_estimate`'s pattern. `None` when the
/// departure doesn't resolve: `route_path`'s ground branch trims by along-route position, so a
/// fabricated placeholder (e.g. `(0.0, 0.0)`) would corrupt crossing detection whenever the arrival
/// or route content still resolves ≥2 anchors on its own (`nav::build_anchors` only skips the
/// *departure* anchor for an unresolvable `dep` — it still resolves the arrival and any enroute
/// fixes) — the caller must skip that prefile, not guess its position.
fn prefile_position(airports: &AirportDb, dep: &str) -> Option<(f64, f64)> {
    airports.get(&dep.to_ascii_uppercase()).copied()
}

/// The polyline `aircraft_route` draws for a connected pilot (#213). `route_path`'s ground branch
/// deliberately collapses to `None` once a ground aircraft has landed at its destination — correct
/// for FCA-crossing detection (there's no crossing left ahead of it), but that same `None` must not
/// turn into an empty `points` array here: this endpoint backs a map polyline + detail popup, and a
/// landed/taxiing aircraft still needs a valid, non-empty track (its own position) rather than
/// silently drawing nothing where the full filed route used to show. `waypoints` (from
/// `full_route_named`, built separately by the caller) already carries the complete filed route for
/// the popup regardless of this fallback, so this only fixes the drawn line.
fn route_display_points(path: Option<Vec<[f64; 2]>>, lat: f64, lon: f64) -> Vec<[f64; 2]> {
    path.unwrap_or_else(|| vec![[lat, lon]])
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

/// Whether the FCA's crossing point falls within its scoped ARTCCs. An empty scope (or no
/// boundary data) means no restriction. Enforced separately from `passes_filters` because it
/// needs the resolved crossing coordinate, not just the flight plan.
fn passes_scope(fca: &FcaBody, airspace: &Boundaries, lat: f64, lon: f64) -> bool {
    if fca.scope.is_empty() || airspace.is_empty() {
        return true;
    }
    fca.scope.iter().any(|z| airspace.contains(z, lat, lon))
}

/// Filed cruise altitude in feet, or `None` when unfiled/unparseable — used for altitude membership
/// so an aircraft whose plan omits an altitude isn't excluded on that basis. Values ≤ 600 are read as
/// flight levels (×100), matching how pilots file `"350"` for FL350.
fn filed_altitude_ft(filed: &str) -> Option<i64> {
    let digits: String = filed.chars().filter(|c| c.is_ascii_digit()).collect();
    let n: i64 = digits.parse().ok()?;
    if n == 0 {
        return None;
    }
    Some(if n <= 600 { n * 100 } else { n })
}

/// Membership filters (dest / origin / fix / altitude). `cur_alt_ft` is the aircraft's live altitude
/// (`None` for a prefile with no position). Altitude membership matches if **either** the filed cruise
/// **or** the current altitude falls in the FCA's band: a climbing aircraft is caught by its filed
/// cruise, and one that filed a bogus-low altitude but is actually cruising in-band is caught by its
/// current altitude. Unknown-on-both ⇒ not excluded. ARTCC scope is enforced separately by
/// `passes_scope` on the crossing.
fn passes_filters(fca: &FcaBody, fp: &FlightPlan, cur_alt_ft: Option<i64>) -> bool {
    if !fca.dests.is_empty() && !fca.dests.iter().any(|d| airport_match(d, &fp.arrival)) {
        return false;
    }
    if !fca.origins.is_empty() && !fca.origins.iter().any(|o| airport_match(o, &fp.departure)) {
        return false;
    }
    if !fca.fixes.is_empty() && !fca.fixes.iter().any(|f| route_has_fix(&fp.route, f)) {
        return false;
    }
    altitude_matches(
        filed_altitude_ft(&fp.altitude),
        cur_alt_ft.filter(|a| *a > 0),
        fca.min_fl,
        fca.max_fl,
    )
}

/// Whether an aircraft's altitude qualifies for an FCA band. Matches if **either** the filed cruise
/// or the current altitude sits within `[min_fl, max_fl]` (either bound may be open). When neither
/// altitude is known, it isn't excluded on altitude. No band at all ⇒ always matches.
fn altitude_matches(
    filed_ft: Option<i64>,
    cur_alt_ft: Option<i64>,
    min_fl: Option<i32>,
    max_fl: Option<i32>,
) -> bool {
    if min_fl.is_none() && max_fl.is_none() {
        return true;
    }
    let lo = min_fl.map(|m| m as i64 * 100);
    let hi = max_fl.map(|m| m as i64 * 100);
    let in_band = |alt: i64| lo.is_none_or(|l| alt >= l) && hi.is_none_or(|h| alt <= h);
    let filed_ok = filed_ft.map(in_band);
    let current_ok = cur_alt_ft.map(in_band);
    match (filed_ok, current_ok) {
        // Known on at least one → must be in band on at least one.
        (Some(a), Some(b)) => a || b,
        (Some(a), None) => a,
        (None, Some(b)) => b,
        // Unknown on both → don't exclude.
        (None, None) => true,
    }
}

fn validate_fca(req: &UpsertFcaRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    if let Some(m) = &req.mode
        && m != "rate"
        && m != "mit"
    {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/fcas",
    tag = "flow",
    responses((status = 200, body = Vec<FcaBody>), (status = 401))
)]
pub async fn list_fcas(State(state): State<AppState>) -> Result<Json<Vec<FcaBody>>, ApiError> {
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
    state.publish(crate::realtime::topic::FCA);
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
    state.publish(crate::realtime::topic::FCA);
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
        state.publish(crate::realtime::topic::FCA);
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
        artcc: row.artcc,
        route: row.route,
        dep: row.dep,
        arr: row.arr,
        points,
        waypoints: to_waypoints(named),
        unresolved,
        updated_at: row.updated_at,
        updated_by: row.updated_by,
    }
}

/// Permission names for route edit/delete scope checks (the caller must hold these for the route's ARTCC).
const ROUTE_UPDATE_PERM: &str = "flow.route.update";
const ROUTE_DELETE_PERM: &str = "flow.route.delete";

#[derive(Deserialize)]
pub struct RoutesQuery {
    /// Scope to one ARTCC's routes (plus the global ones). Omit for every route (the national view).
    artcc: Option<String>,
}

/// Shared map routes, each resolved to a track. With `?artcc=ZDC`, only that ARTCC's routes plus the
/// global (unassigned) ones — how the facility map scopes them. Public (anyone who can view the map).
#[utoipa::path(
    get,
    path = "/api/v1/flow/routes",
    tag = "flow",
    params(("artcc" = Option<String>, Query, description = "Scope to one ARTCC (+ global routes)")),
    responses((status = 200, body = Vec<RouteBody>), (status = 401))
)]
pub async fn list_routes(
    State(state): State<AppState>,
    Query(q): Query<RoutesQuery>,
) -> Result<Json<Vec<RouteBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = flow_repo::norm_artcc(q.artcc.as_deref());
    let rows = flow_repo::list_routes(pool, artcc.as_deref()).await?;
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
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Json(payload): Json<UpsertRouteRequest>,
) -> Result<Json<RouteBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_route(&payload)?;
    // Facility scope: the caller must hold flow.route.update for the route's ARTCC (or nationally —
    // which is also what a global, null-ARTCC route requires).
    let artcc = flow_repo::norm_artcc(payload.artcc.as_deref());
    let scope = principal
        .permission_scope(&state, ROUTE_UPDATE_PERM)
        .await?;
    if !scope.allows(artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }
    let id = flow_repo::create_route(pool, &payload, principal.user_id()).await?;
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
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
    Json(payload): Json<UpsertRouteRequest>,
) -> Result<Json<RouteBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_route(&payload)?;
    let existing = flow_repo::get_route(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Facility scope: the caller must control the route as it is AND where it's headed — so a ZDC
    // editor can't hijack a ZLA route, nor reassign a route into an ARTCC they don't hold.
    let new_artcc = flow_repo::norm_artcc(payload.artcc.as_deref());
    let scope = principal
        .permission_scope(&state, ROUTE_UPDATE_PERM)
        .await?;
    if !scope.allows(existing.artcc.as_deref()) || !scope.allows(new_artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }
    if !flow_repo::update_route(pool, &id, &payload, principal.user_id()).await? {
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
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let existing = flow_repo::get_route(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Facility scope: only someone holding flow.route.delete for the route's ARTCC (or nationally).
    let scope = principal
        .permission_scope(&state, ROUTE_DELETE_PERM)
        .await?;
    if !scope.allows(existing.artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }
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
) -> Result<Json<HashMap<String, i64>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fcas = flow_repo::list_fcas(pool).await?;

    // Clone the snapshot + shared handles and release the feed lock; the per-aircraft route resolution
    // (every pilot × every active FCA) is heavy CPU, so run it on a blocking thread rather than inline
    // on the async workers where a burst of pollers could starve the whole runtime.
    let (snapshot, airports) = feed_view(&state).await;
    let Some(snap) = snapshot else {
        return Ok(Json(fcas.iter().map(|f| (f.id.clone(), 0)).collect()));
    };
    let nav = state.nav.load_full();
    let airspace = state.airspace.clone();

    let counts = tokio::task::spawn_blocking(move || {
        let mut counts: HashMap<String, i64> = fcas.iter().map(|f| (f.id.clone(), 0)).collect();
        let active: Vec<&FcaBody> = fcas
            .iter()
            .filter(|f| f.enabled && f.points.0.len() >= 2)
            .collect();
        if active.is_empty() {
            return counts;
        }
        let airports = airports.as_ref();
        let nav = nav.as_ref();
        let airspace = airspace.as_ref();

        // Resolve each aircraft's route once, then test it against every active FCA.
        let mut tally = |fp: &FlightPlan, lat: f64, lon: f64, hdg: i64, gs: i64, alt: i64| {
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
                if !passes_filters(f, fp, Some(alt)) {
                    continue;
                }
                // Match the metering board: only count crossings within the FCA's ARTCC scope.
                if let Some(cross) = fca::crosses(&path, &f.points.0)
                    && passes_scope(f, airspace, cross.lat, cross.lon)
                    && let Some(c) = counts.get_mut(&f.id)
                {
                    *c += 1;
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
            if let Some(fp) = &pf.flight_plan
                && let Some((lat, lon)) = prefile_position(airports, &fp.departure)
            {
                tally(fp, lat, lon, 0, 0, 0);
            }
        }
        counts
    })
    .await
    .map_err(|_| ApiError::Internal)?;
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
        let path = fca::route_path(
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            p.latitude,
            p.longitude,
            p.heading,
            p.groundspeed,
        );
        let points = route_display_points(path, p.latitude, p.longitude);
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

/// Resolve a batch of filed routes to drawable polylines (for the replay map's route overlay).
/// Read-only nav resolution; excluded from audit despite being a POST (the body is just a list of
/// flights to resolve, not a mutation).
#[utoipa::path(
    post,
    path = "/api/v1/flow/resolve-routes",
    tag = "flow",
    request_body = Vec<ResolveRouteRequest>,
    responses((status = 200, body = Vec<ResolvedRoute>), (status = 401))
)]
pub async fn resolve_routes(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Json(reqs): Json<Vec<ResolveRouteRequest>>,
) -> Json<Vec<ResolvedRoute>> {
    let (_, airports) = feed_view(&state).await;
    let nav_db = state.nav.load_full();
    let nav = nav_db.as_ref();
    let airports = airports.as_ref();
    let out = reqs
        .into_iter()
        .map(|r| {
            let (named, unresolved) =
                fca::full_route_named(nav, airports, &r.dep, &r.arr, &r.route);
            let points = named.iter().map(|(_, lat, lon)| [*lat, *lon]).collect();
            ResolvedRoute {
                callsign: r.callsign,
                points,
                waypoints: to_waypoints(named),
                unresolved,
            }
        })
        .collect();
    Json(out)
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
pub async fn data_status(State(state): State<AppState>) -> Json<DataStatus> {
    Json(build_data_status(&state))
}

/// Public "my flight" lookup — everything currently affecting one callsign: its
/// arrival GDP / ground stop / rate program (with this flight's delay + EDCT) and
/// every FCA it crosses (metered). No auth. `found` is false if it isn't live.
#[utoipa::path(
    get,
    path = "/api/v1/public/flight/{callsign}",
    tag = "public",
    params(("callsign" = String, Path, description = "Aircraft callsign")),
    responses((status = 200, body = FlightAdvisory))
)]
pub async fn flight_advisory(
    State(state): State<AppState>,
    Path(callsign): Path<String>,
) -> Result<Json<FlightAdvisory>, ApiError> {
    let cs = callsign.trim().to_ascii_uppercase();
    Ok(Json(build_flight_advisory(&state, cs).await?))
}

/// Assemble the flight advisory for an already-normalized (upper-cased) callsign — the shared body
/// of `flight_advisory` (public, by callsign) and `my_flight` (session, by CID).
pub(crate) async fn build_flight_advisory(
    state: &AppState,
    cs: String,
) -> Result<FlightAdvisory, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    // Locate the flight in the live feed (clone just the fields we need).
    let (snapshot, airports) = feed_view(state).await;
    let hit = snapshot.as_ref().and_then(|s| {
        s.data
            .pilots
            .iter()
            .find(|p| p.callsign.eq_ignore_ascii_case(&cs))
            .map(|p| {
                (
                    p.latitude,
                    p.longitude,
                    p.altitude,
                    p.groundspeed,
                    p.heading,
                    p.flight_plan.clone(),
                )
            })
    });
    let Some((lat, lon, altitude, groundspeed, heading, fp)) = hit else {
        return Ok(FlightAdvisory {
            callsign: cs,
            found: false,
            ..Default::default()
        });
    };
    let arr = fp
        .as_ref()
        .map(|f| f.arrival.to_ascii_uppercase())
        .unwrap_or_default();
    let airborne = groundspeed >= 50;

    let mut adv = FlightAdvisory {
        callsign: cs.clone(),
        found: true,
        dep: fp.as_ref().map(|f| f.departure.clone()).unwrap_or_default(),
        arr: arr.clone(),
        aircraft_type: fp
            .as_ref()
            .map(|f| f.aircraft_short.clone())
            .unwrap_or_default(),
        status: if airborne { "airborne" } else { "ground" }.to_string(),
        altitude,
        groundspeed,
        lat,
        lon,
        heading,
        ..Default::default()
    };

    let mut delays: Vec<i64> = Vec::new();
    let mut edcts: Vec<DateTime<Utc>> = Vec::new();

    if !arr.is_empty() {
        // Arrival GDP (+ this flight's frozen slot, if controlled).
        if let Some((gid, aar, start, end)) = public_repo::gdp_for_airport(pool, &arr).await? {
            let (controlled, edct, cta, delay) =
                match public_repo::gdp_slot_for(pool, &gid, &cs).await? {
                    Some((edct, cta, d)) => (true, edct, Some(cta), d as i64),
                    None => (false, None, None, 0),
                };
            if delay > 0 {
                delays.push(delay);
            }
            if let Some(e) = edct {
                edcts.push(e);
            }
            adv.gdp = Some(FlightGdp {
                airport: arr.clone(),
                aar,
                start_time: start,
                end_time: end,
                controlled,
                edct,
                cta,
                delay_min: delay,
            });
        }

        // Arrival ground stop.
        if let Some((scope, until)) = public_repo::ground_stop_for_airport(pool, &arr).await? {
            adv.ground_stop = Some(FlightGroundStop {
                airport: arr.clone(),
                scope,
                until,
            });
        }

        // Arrival rate program — this flight's metered delay from the live flow.
        let flow = crate::handlers::feed::flow_for(state, pool, &arr).await?;
        if let (Some(aar), Some(ff)) = (
            flow.aar,
            flow.flights
                .iter()
                .find(|f| f.callsign.eq_ignore_ascii_case(&cs)),
        ) {
            if ff.delay_min > 0 {
                delays.push(ff.delay_min);
            }
            adv.rate_program = Some(FlightProgram {
                airport: arr.clone(),
                aar,
                delay_min: ff.delay_min,
                sta: ff.sta,
                cfr: ff.cfr,
            });
        }
    }

    // FCA crossings — meter only the enabled FCAs this flight actually crosses.
    if snapshot.is_some() {
        let nav = state.nav.load_full();
        let now = Utc::now();
        // Resolve this flight's route once for the cheap crossing pre-filter, so we don't run
        // the full per-FCA metering (which resolves every matching pilot's route) for FCAs it
        // never crosses.
        let path = fp.as_ref().and_then(|f| {
            fca::route_path(
                nav.as_ref(),
                airports.as_ref(),
                &f.departure,
                &f.arrival,
                &f.route,
                lat,
                lon,
                heading,
                groundspeed,
            )
        });
        for fca in flow_repo::list_fcas(pool)
            .await?
            .into_iter()
            .filter(|f| f.enabled && f.points.0.len() >= 2)
        {
            // Same predicate build_candidates uses to include this flight: filters + a scoped
            // crossing. If it doesn't cross, metering this FCA can't produce a slot for it.
            let crosses = match (&path, &fp) {
                (Some(p), Some(plan)) => {
                    passes_filters(&fca, plan, Some(altitude))
                        && fca::crosses(p, &fca.points.0).is_some_and(|c| {
                            passes_scope(&fca, state.airspace.as_ref(), c.lat, c.lon)
                        })
                }
                _ => false,
            };
            if !crosses {
                continue;
            }
            let releases = load_releases(pool, &fca.id).await?;
            let (fca_id, fca_name, fca_color) =
                (fca.id.clone(), fca.name.clone(), fca.color.clone());
            let metered = metered_flights(state, fca, releases, now, false).await?;
            if let Some(f) = metered
                .into_iter()
                .find(|f| f.callsign.eq_ignore_ascii_case(&cs))
            {
                if f.delay_min > 0 {
                    delays.push(f.delay_min);
                }
                if let Some(e) = f.edct {
                    edcts.push(e);
                }
                adv.fcas.push(FlightFcaCrossing {
                    fca_id,
                    fca_name,
                    color: fca_color,
                    cross_time: f.cross_time,
                    delay_min: f.delay_min,
                    edct: f.edct,
                    seq: Some(f.seq),
                });
            }
        }
    }

    adv.total_delay_min = delays.into_iter().max().unwrap_or(0);
    adv.edct = edcts.into_iter().max();
    Ok(adv)
}

/// Session-scoped "my flight" — resolve the caller's live flight by matching their VATSIM CID
/// against the feed snapshot, then assemble the same advisory `flight_advisory` returns. `found` is
/// false when the caller isn't connected (or has no flight plan) under their CID.
#[utoipa::path(
    get,
    path = "/api/v1/me/flight",
    tag = "public",
    responses((status = 200, body = FlightAdvisory), (status = 401))
)]
pub async fn my_flight(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<Json<FlightAdvisory>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let cs = {
        let (snapshot, _) = feed_view(&state).await;
        snapshot.as_ref().and_then(|s| {
            s.data
                .pilots
                .iter()
                .find(|p| p.cid as i64 == user.cid)
                .map(|p| p.callsign.to_ascii_uppercase())
        })
    };
    match cs {
        Some(cs) => Ok(Json(build_flight_advisory(&state, cs).await?)),
        None => Ok(Json(FlightAdvisory {
            found: false,
            ..Default::default()
        })),
    }
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
) -> Result<Json<crate::feed::coverage::CoverageReport>, ApiError> {
    let (snapshot, airports) = feed_view(&state).await;
    let snap = snapshot.ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(crate::feed::coverage::analyze(
        state.nav.load_full().as_ref(),
        airports.as_ref(),
        &snap.data,
    )))
}

#[derive(Deserialize)]
pub struct ValidateFixesQuery {
    /// Space/comma-separated fix tokens to check against the nav database.
    fixes: Option<String>,
}

/// Report which of the submitted route-fix tokens aren't real nav fixes — so the FCA editor can flag
/// typos (e.g. `MLLETT` for `MLLET`) that would silently exclude matching traffic.
#[utoipa::path(
    get, path = "/api/v1/flow/validate-fixes", tag = "flow",
    params(("fixes" = Option<String>, Query, description = "Space/comma-separated fix tokens")),
    responses((status = 200, body = FixValidationBody), (status = 401))
)]
pub async fn validate_fixes(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Query(q): Query<ValidateFixesQuery>,
) -> Json<FixValidationBody> {
    let nav = state.nav.load_full();
    // Without nav data loaded we can't judge anything — flag nothing rather than everything.
    if nav.fix_count() == 0 {
        return Json(FixValidationBody {
            unknown: Vec::new(),
        });
    }
    let mut seen = HashSet::new();
    let unknown = q
        .fixes
        .unwrap_or_default()
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| t.trim().to_ascii_uppercase())
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .filter(|t| !nav.knows(t))
        .collect();
    Json(FixValidationBody { unknown })
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
pub async fn list_traffic(State(state): State<AppState>) -> Json<Vec<TrafficAircraft>> {
    let snapshot = state.feed.read().await.snapshot.clone();
    let aircraft = snapshot
        .as_ref()
        .map(|snap| traffic_from(&snap.data))
        .unwrap_or_default();
    Json(aircraft)
}

#[derive(Deserialize)]
pub struct ProjectQuery {
    /// How far ahead to project, in seconds (0..=5400).
    offset_sec: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/traffic/projected",
    tag = "flow",
    params(("offset_sec" = i64, Query, description = "Seconds ahead to project (0-5400)")),
    responses((status = 200, body = Vec<TrafficAircraft>), (status = 400), (status = 401))
)]
pub async fn projected_traffic(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
) -> Result<Json<Vec<TrafficAircraft>>, ApiError> {
    if !(0..=MAX_PROJECTION_SEC).contains(&q.offset_sec) {
        return Err(ApiError::BadRequest);
    }
    let (snapshot, airports) = feed_view(&state).await;
    let Some(snap) = snapshot else {
        return Ok(Json(Vec::new()));
    };
    let nav = state.nav.load_full();
    let profiles = state.aircraft_profiles.load_full();
    let winds = state.winds.load_full();
    let aircraft = tokio::task::spawn_blocking(move || {
        project_traffic(
            &snap.data,
            nav.as_ref(),
            airports.as_ref(),
            profiles.as_ref(),
            winds.as_ref(),
            q.offset_sec,
        )
    })
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(Json(aircraft))
}

/// Map a snapshot's pilots to the lightweight map-traffic shape (drops position-less aircraft).
/// Pure of the live feed so the historical replay can reuse it against a reconstructed snapshot.
pub(crate) fn traffic_from(data: &VatsimData) -> Vec<TrafficAircraft> {
    data.pilots
        .iter()
        .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
        .map(|p| {
            let fp = p.flight_plan.as_ref();
            let arr = fp.map(|f| f.arrival.clone()).unwrap_or_default();
            // Enrich for the facility map's client-side color rules: STAR/gate (base name), wake,
            // flight rules, and filed cruise altitude. Cheap per-pilot string parsing.
            let (star, wake, flight_rules, filed_alt) = match fp {
                Some(f) => (
                    crate::feed::flow::arrival_gate(&f.route, &arr)
                        .map(|g| crate::feed::runway::star_base(&g)),
                    f.aircraft_type_wake().1,
                    f.flight_rules.clone(),
                    trajectory::parse_alt_ft(&f.altitude) as i32,
                ),
                None => (None, String::new(), String::new(), 0),
            };
            TrafficAircraft {
                callsign: p.callsign.clone(),
                lat: p.latitude,
                lon: p.longitude,
                heading: p.heading,
                gs: p.groundspeed,
                alt: p.altitude,
                dep: fp.map(|f| f.departure.clone()).unwrap_or_default(),
                arr,
                actype: fp.map(|f| f.aircraft_short.clone()).unwrap_or_default(),
                star,
                wake,
                flight_rules,
                filed_alt,
            }
        })
        .collect()
}

/// Cap on how far ahead the prediction scrubber (#226) can project — bounds the compute cost of an
/// arbitrary client-picked `offset_sec` and matches the issue's own suggested window.
pub(crate) const MAX_PROJECTION_SEC: i64 = 90 * 60;

/// `traffic_from`'s forward-time counterpart (#226): every airborne pilot is projected
/// `offset_sec` ahead along its own resolved route, using the same shared trajectory/ETA model
/// FCA metering and the arrival ladder use — never a separate, inconsistent prediction. Ground
/// aircraft are left parked (not yet departed, nothing to project) and an aircraft whose route
/// can't be resolved past its dep/arr endpoints is dropped rather than shown at a fabricated
/// position, mirroring how `route_path` itself signals "can't resolve" with `None`.
pub(crate) fn project_traffic(
    data: &VatsimData,
    nav: &NavData,
    airports: &AirportDb,
    profiles: &trajectory::ProfileTable,
    winds: &Winds,
    offset_sec: i64,
) -> Vec<TrafficAircraft> {
    let offset_sec = offset_sec as f64;
    data.pilots
        .iter()
        .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
        .filter_map(|p| {
            let fp = p.flight_plan.as_ref()?;
            let arr = fp.arrival.clone();
            let airborne = p.groundspeed > 60 && p.altitude > 300;

            let (star, wake, flight_rules, filed_alt) = (
                feed_flow::arrival_gate(&fp.route, &arr)
                    .map(|g| crate::feed::runway::star_base(&g)),
                fp.aircraft_type_wake().1,
                fp.flight_rules.clone(),
                trajectory::parse_alt_ft(&fp.altitude) as i32,
            );
            let actype = fp.aircraft_short.clone();
            let dep = fp.departure.clone();

            if !airborne || offset_sec <= 0.0 {
                // Not yet departed, or T=0: nothing to project, show where it actually is.
                return Some(TrafficAircraft {
                    callsign: p.callsign.clone(),
                    lat: p.latitude,
                    lon: p.longitude,
                    heading: p.heading,
                    gs: p.groundspeed,
                    alt: p.altitude,
                    dep,
                    arr,
                    actype,
                    star,
                    wake,
                    flight_rules,
                    filed_alt,
                });
            }

            let path = fca::route_path(
                nav,
                airports,
                &dep,
                &arr,
                &fp.route,
                p.latitude,
                p.longitude,
                p.heading,
                p.groundspeed,
            )?;
            let route_len_nm = predict::path_len_nm(&path);
            let (ty, wake_code) = fp.aircraft_type_wake();
            let profile = profiles.resolve(&ty, &wake_code);
            let cruise_ft = trajectory::parse_alt_ft(&fp.altitude);
            let cruise_tas = trajectory::capped_cruise_tas(
                feed_flow::parse_tas(&fp.cruise_tas),
                cruise_ft,
                profile,
            );
            let headwind = winds.route_headwind(&path, p.altitude as f64);
            let vp = trajectory::VerticalProfile::build(
                p.altitude as f64,
                route_len_nm,
                0.0,
                cruise_ft,
                cruise_tas,
                profile,
                headwind,
            );
            let target_d = vp.distance_after(route_len_nm, offset_sec);
            let ahead_nm = (route_len_nm - target_d).max(0.0);
            let (pos, heading) = fca::point_and_heading_at(&path, ahead_nm);
            let alt = vp.alt_at(target_d).round() as i64;

            Some(TrafficAircraft {
                callsign: p.callsign.clone(),
                lat: pos[0],
                lon: pos[1],
                heading: heading.round() as i64,
                gs: p.groundspeed,
                alt,
                dep,
                arr,
                actype,
                star,
                wake,
                flight_rules,
                filed_alt,
            })
        })
        .collect()
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
        delay_sec: 0,
        delay_nm: 0,
        seq: 0,
        edct: rel.and_then(|(_, e)| DateTime::from_timestamp_millis(*e)),
        released: rel.is_some(),
        groundspeed: gs,
        altitude: alt,
        heading: hdg,
        debug: None,
    }
}

/// Build the debug detail for one crossing flight (only when debug mode is on): the resolved
/// profile, the speeds/wind used, any unresolvable filed-route tokens, and (when ground) the
/// learned taxi/pushback derivation (#164 sub-issue F) — `taxi` is `None` for an airborne flight.
#[allow(clippy::too_many_arguments)]
fn fca_debug(
    profiles: &trajectory::ProfileTable,
    ty: &str,
    wake: &str,
    cruise_tas: f64,
    cruise_alt: f64,
    headwind: Option<f64>,
    nav: &NavData,
    airports: &AirportDb,
    dep: &str,
    arr: &str,
    route: &str,
    taxi: Option<&feed_flow::GroundAllowanceBreakdown>,
) -> crate::models::FcaFlightDebug {
    let (_waypoints, unresolved) = fca::full_route_named(nav, airports, dep, arr, route);
    crate::models::FcaFlightDebug {
        profile: profiles.resolve_label(ty, wake),
        cruise_tas: cruise_tas.round() as i64,
        cruise_alt: cruise_alt.round() as i64,
        headwind: headwind.map(|h| h.round() as i64),
        unresolved,
        taxi_estimate: taxi.map(|t| crate::models::TaxiEstimateDebug {
            gate: t.gate_id.clone(),
            runway: t.runway.clone(),
            pushback_sec: t.pushback.value_sec.round() as i64,
            pushback_tier: t.pushback.tier.label().to_string(),
            pushback_samples: t.pushback.sample_count as i64,
            taxi_sec: t.taxi.value_sec.round() as i64,
            taxi_tier: t.taxi.tier.label().to_string(),
            taxi_samples: t.taxi.sample_count as i64,
        }),
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
    profiles: &trajectory::ProfileTable,
    releases: &ReleaseMap,
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    now: DateTime<Utc>,
    debug: bool,
) -> (Vec<FcaFlight>, Vec<fca::MeterInput>) {
    let pts = fca.points.0.clone();
    let mut flights = Vec::new();
    let mut metas = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let airborne = p.groundspeed >= 50;
        if !passes_filters(fca, fp, Some(p.altitude)) {
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
        let Some(cross) = fca::crosses(&path, &pts) else {
            continue;
        };
        if !passes_scope(fca, airspace, cross.lat, cross.lon) {
            continue;
        }
        let (ty, wake) = fp.aircraft_type_wake();
        let profile = profiles.resolve(&ty, &wake);
        let cruise = trajectory::parse_alt_ft(&fp.altitude);
        let filed_tas = fp.cruise_tas.parse().unwrap_or(0.0);
        let cruise_tas = trajectory::capped_cruise_tas(filed_tas, cruise, profile);
        let headwind = winds.route_headwind(&path, cruise);
        let route_len = predict::path_len_nm(&path);
        let dep = fp.departure.to_ascii_uppercase();
        // Airborne pilots never apply the allowance (`eta_along_route`'s `!airborne` gate) — skip
        // the lookup for them and pass 0.0.
        let ground_taxi = if airborne {
            None
        } else {
            let aircraft = (!fp.aircraft_short.is_empty()).then_some(fp.aircraft_short.as_str());
            Some(feed_flow::resolve_ground_allowance(
                gates,
                runways,
                taxi_samples,
                &dep,
                aircraft,
                Some((p.latitude, p.longitude, p.heading, p.groundspeed)),
            ))
        };
        let allowance = ground_taxi.as_ref().map(|b| b.total_sec()).unwrap_or(0.0);
        let eta = predict::eta_along_route(
            airborne,
            route_len,
            cross.along_nm,
            p.altitude as f64,
            cruise,
            cruise_tas,
            profile,
            headwind,
            allowance,
            now,
        );
        let rel = releases.get(&p.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.timestamp_millis(),
            airborne,
            cross_speed: trajectory::effective_gs(cruise_tas, headwind).max(120.0),
            frozen_ms: rel.map(|(cta, _)| *cta),
        });
        let mut flight = fca_flight(
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
        );
        if debug {
            flight.debug = Some(fca_debug(
                profiles,
                &ty,
                &wake,
                cruise_tas,
                cruise,
                headwind,
                nav,
                airports,
                &fp.departure,
                &fp.arrival,
                &fp.route,
                ground_taxi.as_ref(),
            ));
        }
        flights.push(flight);
    }

    for pf in &data.prefiles {
        let Some(fp) = &pf.flight_plan else { continue };
        if !passes_filters(fca, fp, None) {
            continue;
        }
        let Some((dep_lat, dep_lon)) = prefile_position(airports, &fp.departure) else {
            continue;
        };
        let Some(path) = fca::route_path(
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            dep_lat,
            dep_lon,
            0,
            0,
        ) else {
            continue;
        };
        let Some(cross) = fca::crosses(&path, &pts) else {
            continue;
        };
        if !passes_scope(fca, airspace, cross.lat, cross.lon) {
            continue;
        }
        let (ty, wake) = fp.aircraft_type_wake();
        let profile = profiles.resolve(&ty, &wake);
        let cruise = trajectory::parse_alt_ft(&fp.altitude);
        let filed_tas = fp.cruise_tas.parse().unwrap_or(0.0);
        let cruise_tas = trajectory::capped_cruise_tas(filed_tas, cruise, profile);
        let headwind = winds.route_headwind(&path, cruise);
        let route_len = predict::path_len_nm(&path);
        // A prefile has no live position — gate/runway matching is skipped, falling to the
        // airport/default tier.
        let dep = fp.departure.to_ascii_uppercase();
        let aircraft = (!fp.aircraft_short.is_empty()).then_some(fp.aircraft_short.as_str());
        let ground_taxi =
            feed_flow::resolve_ground_allowance(gates, runways, taxi_samples, &dep, aircraft, None);
        let allowance = ground_taxi.total_sec();
        let eta = predict::eta_along_route(
            false,
            route_len,
            cross.along_nm,
            0.0,
            cruise,
            cruise_tas,
            profile,
            headwind,
            allowance,
            now,
        );
        let rel = releases.get(&pf.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.timestamp_millis(),
            airborne: false,
            cross_speed: trajectory::effective_gs(cruise_tas, headwind).max(120.0),
            frozen_ms: rel.map(|(cta, _)| *cta),
        });
        let mut flight = fca_flight(
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
        );
        if debug {
            flight.debug = Some(fca_debug(
                profiles,
                &ty,
                &wake,
                cruise_tas,
                cruise,
                headwind,
                nav,
                airports,
                &fp.departure,
                &fp.arrival,
                &fp.route,
                Some(&ground_taxi),
            ));
        }
        flights.push(flight);
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
    for ((f, m), input) in flights.iter_mut().zip(&metered).zip(metas) {
        f.cross_time = DateTime::from_timestamp_millis(m.sched_ms);
        f.delay_min = (m.delay_sec + 30) / 60;
        f.delay_sec = m.delay_sec;
        // Delay as extra track miles at the predicted crossing speed (matches the flow the
        // controller sees): how much further back this aircraft must sit to hold separation.
        f.delay_nm = (m.delay_sec as f64 / 3600.0 * input.cross_speed).round() as i64;
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

/// Build the metered crossing list for one FCA, running the CPU-heavy route resolution + sequencing
/// on a **blocking thread**. `build_candidates` resolves every matching aircraft's route against the
/// FCA — pure CPU with no `.await` — so running it inline on the async workers lets a burst of polling
/// clients starve the whole runtime (health check + websocket keepalive included) and the process
/// appears hung. `spawn_blocking` keeps it off the async threads. All shared state is `Arc`, so the
/// closure owns cheap clones.
async fn metered_flights(
    state: &AppState,
    fca: FcaBody,
    releases: ReleaseMap,
    now: DateTime<Utc>,
    debug: bool,
) -> Result<Vec<FcaFlight>, ApiError> {
    if fca.points.0.len() < 2 {
        return Ok(Vec::new());
    }
    let (snapshot, airports) = feed_view(state).await;
    let Some(snap) = snapshot else {
        return Ok(Vec::new());
    };
    let nav = state.nav.load_full();
    let winds = state.winds.load_full();
    let aircraft_profiles = state.aircraft_profiles.load_full();
    let airspace = state.airspace.clone();
    let gates = state.gates.load_full();
    let runways = state.runways.clone();
    let taxi_estimate_samples = state.taxi_estimate_samples.load_full();
    tokio::task::spawn_blocking(move || {
        let (flights, metas) = build_candidates(
            &fca,
            &snap.data,
            airports.as_ref(),
            nav.as_ref(),
            airspace.as_ref(),
            winds.as_ref(),
            aircraft_profiles.as_ref(),
            &releases,
            gates.as_ref(),
            runways.as_ref(),
            taxi_estimate_samples.as_ref(),
            now,
            debug,
        );
        finalize(&fca, flights, &metas)
    })
    .await
    .map_err(|_| ApiError::Internal)
}

/// Query for the FCA traffic endpoint.
#[derive(Deserialize)]
pub struct TrafficQuery {
    /// When true, each flight carries a `debug` block (profile used, speeds/wind, unresolved route
    /// tokens) for the client's debug mode.
    #[serde(default)]
    debug: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/fcas/{id}/traffic",
    tag = "flow",
    params(
        ("id" = String, Path, description = "FCA id"),
        ("debug" = Option<bool>, Query, description = "Include per-flight ETA/metering debug detail")
    ),
    responses((status = 200, body = Vec<FcaFlight>), (status = 401), (status = 404))
)]
pub async fn fca_traffic(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TrafficQuery>,
) -> Result<Json<Vec<FcaFlight>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = flow_repo::get_fca(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let releases = load_releases(pool, &id).await?;
    let now = Utc::now();
    Ok(Json(
        metered_flights(&state, fca, releases, now, q.debug).await?,
    ))
}

/// Scope for the IDST board — comma-separated airport, TRACON, and ARTCC codes.
#[derive(Deserialize)]
pub struct IdstQuery {
    airports: Option<String>,
    tracons: Option<String>,
    artccs: Option<String>,
}

fn split_codes(s: &Option<String>) -> Vec<String> {
    s.as_deref()
        .unwrap_or("")
        .split(',')
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| !c.is_empty())
        .collect()
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/idst",
    tag = "flow",
    params(
        ("airports" = Option<String>, Query, description = "Comma-separated airport ICAOs"),
        ("tracons" = Option<String>, Query, description = "Comma-separated TRACON ids"),
        ("artccs" = Option<String>, Query, description = "Comma-separated ARTCC ids")
    ),
    responses((status = 200, body = IdstResponse), (status = 401))
)]
pub async fn list_idst(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Query(q): Query<IdstQuery>,
) -> Result<Json<IdstResponse>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let now = Utc::now();
    let empty = |now| {
        Json(IdstResponse {
            unscheduled: Vec::new(),
            released: Vec::new(),
            metered_count: 0,
            as_of: now,
        })
    };

    // Resolve the scope (airports/TRACONs/ARTCCs) to the union of member airport ICAOs.
    let codes: Vec<String> = split_codes(&q.airports)
        .into_iter()
        .chain(split_codes(&q.tracons))
        .chain(split_codes(&q.artccs))
        .collect();
    let airports: HashSet<String> = {
        let map = state.facilities.read().await;
        codes
            .iter()
            .flat_map(|c| facilities::member_airports(&map, c))
            .map(|a| a.to_ascii_uppercase())
            .collect()
    };
    if airports.is_empty() {
        return Ok(empty(now));
    }

    // Gather each enabled FCA with its releases (async DB) first, then hand the whole per-FCA metering
    // loop to a blocking thread — it resolves every ground departure's route for every FCA, which is
    // heavy CPU that must stay off the async workers (see `metered_flights`).
    let mut fca_releases: Vec<(FcaBody, ReleaseMap)> = Vec::new();
    for fca in flow_repo::list_fcas(pool)
        .await?
        .into_iter()
        .filter(|f| f.enabled && f.points.0.len() >= 2)
    {
        let releases = load_releases(pool, &fca.id).await?;
        fca_releases.push((fca, releases));
    }
    let (snapshot, ap) = feed_view(&state).await;
    let Some(snap) = snapshot else {
        return Ok(empty(now));
    };
    let nav = state.nav.load_full();
    let winds = state.winds.load_full();
    let aircraft_profiles = state.aircraft_profiles.load_full();
    let airspace = state.airspace.clone();
    let gates = state.gates.load_full();
    let runways = state.runways.clone();
    let taxi_estimate_samples = state.taxi_estimate_samples.load_full();

    let (mut unscheduled, mut released) = tokio::task::spawn_blocking(move || {
        let mut unscheduled: Vec<IdstFlight> = Vec::new();
        let mut released: Vec<IdstFlight> = Vec::new();
        // One row per (metering FCA, ground departure in scope).
        for (fca, releases) in &fca_releases {
            let (flights, metas) = build_candidates(
                fca,
                &snap.data,
                ap.as_ref(),
                nav.as_ref(),
                airspace.as_ref(),
                winds.as_ref(),
                aircraft_profiles.as_ref(),
                releases,
                gates.as_ref(),
                runways.as_ref(),
                taxi_estimate_samples.as_ref(),
                now,
                false,
            );
            for f in finalize(fca, flights, &metas) {
                if (f.status != "ground" && f.status != "proposed")
                    || !airports.contains(&f.dep.to_ascii_uppercase())
                {
                    continue;
                }
                // For released flights the frozen wheels-up; otherwise an *advisory* EDCT — the
                // wheels-up that would hit the metered crossing, backing out the modeled transit
                // (eta − now) from the metered CTA. Lets the controller see the proposed release.
                let edct = if f.released {
                    f.edct
                } else if let (Some(cta), Some(eta)) = (f.cross_time, f.eta) {
                    let transit_ms = eta.timestamp_millis() - now.timestamp_millis();
                    DateTime::from_timestamp_millis(cta.timestamp_millis() - transit_ms)
                } else {
                    None
                };
                let item = IdstFlight {
                    callsign: f.callsign,
                    dep: f.dep,
                    arr: f.arr,
                    aircraft_type: f.aircraft_type,
                    status: f.status,
                    fca_id: fca.id.clone(),
                    fca_name: fca.name.clone(),
                    seq: f.seq,
                    delay_min: f.delay_min,
                    cross_time: f.cross_time,
                    edct,
                    released: f.released,
                };
                if item.released {
                    released.push(item);
                } else {
                    unscheduled.push(item);
                }
            }
        }
        (unscheduled, released)
    })
    .await
    .map_err(|_| ApiError::Internal)?;
    // Unscheduled by metered crossing (soonest first); released by frozen wheels-up.
    unscheduled.sort_by_key(|f| {
        f.cross_time
            .map(|t| t.timestamp_millis())
            .unwrap_or(i64::MAX)
    });
    released.sort_by_key(|f| f.edct.map(|t| t.timestamp_millis()).unwrap_or(i64::MAX));

    let metered_count = (unscheduled.len() + released.len()) as i64;
    Ok(Json(IdstResponse {
        unscheduled,
        released,
        metered_count,
        as_of: now,
    }))
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
            state.aircraft_profiles.load_full().as_ref(),
            &releases,
            state.gates.load_full().as_ref(),
            state.runways.as_ref(),
            state.taxi_estimate_samples.load_full().as_ref(),
            now,
            false,
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
    state.publish(crate::realtime::topic::RELEASE);

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
    state.publish(crate::realtime::topic::RELEASE);
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
            state.aircraft_profiles.load_full().as_ref(),
            &releases,
            state.gates.load_full().as_ref(),
            state.runways.as_ref(),
            state.taxi_estimate_samples.load_full().as_ref(),
            now,
            false,
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
    state.publish(crate::realtime::topic::FCA);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod filed_altitude_tests {
    use super::{altitude_matches, filed_altitude_ft};

    #[test]
    fn altitude_band_matches_filed_or_current() {
        // Band FL240–600.
        let band = (Some(240), Some(600));
        // UAL3430: filed FL130 (out) but cruising FL255 (in) → matches on current.
        assert!(altitude_matches(Some(13000), Some(25588), band.0, band.1));
        // Original climber: filed FL350 (in) but currently FL180 (out) → matches on filed.
        assert!(altitude_matches(Some(35000), Some(18000), band.0, band.1));
        // Genuinely out of band on both (low GA): excluded.
        assert!(!altitude_matches(Some(8000), Some(8000), band.0, band.1));
        // Overflight above the band on both: excluded.
        assert!(!altitude_matches(
            Some(45000),
            Some(45000),
            Some(200),
            Some(300)
        ));
        // Prefile with only a filed altitude in band: matches.
        assert!(altitude_matches(Some(35000), None, band.0, band.1));
        // Nothing known → not excluded.
        assert!(altitude_matches(None, None, band.0, band.1));
        // No band at all → always matches.
        assert!(altitude_matches(Some(8000), Some(8000), None, None));
    }

    #[test]
    fn parses_flight_levels_and_feet() {
        assert_eq!(filed_altitude_ft("350"), Some(35000)); // bare FL
        assert_eq!(filed_altitude_ft("FL350"), Some(35000));
        assert_eq!(filed_altitude_ft("35000"), Some(35000)); // explicit feet
        assert_eq!(filed_altitude_ft("5000"), Some(5000)); // low feet (> 600, not an FL)
        assert_eq!(filed_altitude_ft("600"), Some(60000)); // <= 600 → FL600
    }

    #[test]
    fn unfiled_is_none_so_it_is_not_excluded() {
        assert_eq!(filed_altitude_ft(""), None);
        assert_eq!(filed_altitude_ft("VFR"), None);
        assert_eq!(filed_altitude_ft("0"), None);
    }
}

/// #226's forward prediction scrubber, exercised against the real bundled nav db so the projected
/// aircraft actually walks a resolved route rather than a synthetic one.
#[cfg(test)]
mod project_traffic_tests {
    use std::collections::HashMap;

    use super::{VatsimData, project_traffic};
    use crate::feed::{
        nav::NavData, trajectory::ProfileTable, vatsim::FlightPlan, vatsim::Pilot, vatsim::Prefile,
        winds::Winds,
    };

    fn airports() -> HashMap<String, (f64, f64)> {
        HashMap::from([
            ("KJFK".to_string(), (40.64, -73.78)),
            ("KDCA".to_string(), (38.85, -77.04)),
        ])
    }

    /// Airborne B738 just south of KJFK tracking SW down the coast, filed KJFK -> KDCA via a
    /// route the bundled nav db resolves (mirrors predict.rs's own resolved-route tests).
    fn airborne_pilot() -> Pilot {
        Pilot {
            callsign: "TEST1".into(),
            latitude: 40.2,
            longitude: -74.0,
            altitude: 24_000,
            groundspeed: 400,
            heading: 220,
            flight_plan: Some(FlightPlan {
                departure: "KJFK".into(),
                arrival: "KDCA".into(),
                route: "RBV WHITE SIE".into(),
                aircraft_short: "B738".into(),
                cruise_tas: "440".into(),
                altitude: "35000".into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn zero_offset_matches_the_live_position() {
        let data = VatsimData {
            pilots: vec![airborne_pilot()],
            ..Default::default()
        };
        let out = project_traffic(
            &data,
            &NavData::load(),
            &airports(),
            &ProfileTable::default(),
            &Winds::default(),
            0,
        );
        assert_eq!(out.len(), 1);
        assert!((out[0].lat - 40.2).abs() < 1e-9);
        assert!((out[0].lon - -74.0).abs() < 1e-9);
    }

    #[test]
    fn an_airborne_aircraft_moves_forward_over_time() {
        let data = VatsimData {
            pilots: vec![airborne_pilot()],
            ..Default::default()
        };
        let out = project_traffic(
            &data,
            &NavData::load(),
            &airports(),
            &ProfileTable::default(),
            &Winds::default(),
            20 * 60,
        );
        assert_eq!(out.len(), 1);
        let moved = crate::feed::flow::gc_dist(40.2, -74.0, out[0].lat, out[0].lon);
        assert!(
            moved > 50.0,
            "expected the aircraft to have moved meaningfully in 20 minutes, moved {moved}nm"
        );
    }

    #[test]
    fn a_ground_aircraft_stays_parked() {
        let mut p = airborne_pilot();
        p.groundspeed = 0;
        p.altitude = 0;
        let data = VatsimData {
            pilots: vec![p],
            ..Default::default()
        };
        let out = project_traffic(
            &data,
            &NavData::load(),
            &airports(),
            &ProfileTable::default(),
            &Winds::default(),
            30 * 60,
        );
        assert_eq!(out.len(), 1);
        assert!((out[0].lat - 40.2).abs() < 1e-9);
        assert!((out[0].lon - -74.0).abs() < 1e-9);
    }

    #[test]
    fn an_unresolvable_route_is_dropped_not_fabricated() {
        let mut p = airborne_pilot();
        // Neither KJFK nor KDCA is in this test's tiny airport cache, and the route fixes won't
        // resolve either — route_path returns None, so this aircraft must not appear at all.
        p.flight_plan.as_mut().unwrap().departure = "ZZZZ".into();
        p.flight_plan.as_mut().unwrap().arrival = "YYYY".into();
        p.flight_plan.as_mut().unwrap().route = "".into();
        let data = VatsimData {
            pilots: vec![p],
            ..Default::default()
        };
        let out = project_traffic(
            &data,
            &NavData::load(),
            &HashMap::new(),
            &ProfileTable::default(),
            &Winds::default(),
            20 * 60,
        );
        assert!(
            out.is_empty(),
            "an unresolvable route must be dropped, not shown at a fabricated position"
        );
    }

    #[test]
    fn prefiles_are_never_projected_matching_the_live_traffic_feed() {
        // traffic_from() itself never includes prefiles (no live position to show) — project_traffic
        // must not start doing so either.
        let data = VatsimData {
            prefiles: vec![Prefile {
                callsign: "TEST2".into(),
                flight_plan: Some(FlightPlan {
                    departure: "KJFK".into(),
                    arrival: "KDCA".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = project_traffic(
            &data,
            &NavData::load(),
            &airports(),
            &ProfileTable::default(),
            &Winds::default(),
            600,
        );
        assert!(out.is_empty());
    }
}

#[cfg(test)]
mod prefile_position_tests {
    use std::collections::HashMap;

    use super::prefile_position;

    /// Regression (#213): a prefile has no live position, so `fca_counts`/`build_candidates` must
    /// resolve its departure airport's real coordinates — not fall through to a fabricated `(0.0,
    /// 0.0)`, which `route_path`'s ground branch now trims by along-route position instead of
    /// ignoring, corrupting crossing detection and distance for a route that happens to project
    /// Null Island onto its far end (see `feed::fca`'s own regression tests for that mechanism).
    #[test]
    fn resolves_the_real_departure_airport_not_null_island() {
        let airports: crate::feed::airports::AirportDb =
            HashMap::from([("KJFK".to_string(), (40.64, -73.78))]);
        assert_eq!(prefile_position(&airports, "KJFK"), Some((40.64, -73.78)));
        // Case-insensitive, matching route_path's own uppercasing.
        assert_eq!(prefile_position(&airports, "kjfk"), Some((40.64, -73.78)));
    }

    #[test]
    fn is_none_for_an_unresolvable_airport() {
        // `nav::build_anchors` only skips the *departure* anchor for an unresolvable `dep` — it
        // still resolves the arrival airport and any enroute fixes independently, so a fabricated
        // position here could still reach route_path's ground trimming. The caller must skip this
        // prefile instead of guessing a position.
        let airports: crate::feed::airports::AirportDb = HashMap::new();
        assert_eq!(prefile_position(&airports, "ZZZZ"), None);
    }
}

#[cfg(test)]
mod route_display_points_tests {
    use super::route_display_points;

    #[test]
    fn returns_the_resolved_path_unchanged() {
        let path = vec![[40.0, -73.0], [39.0, -74.0]];
        assert_eq!(route_display_points(Some(path.clone()), 0.0, 0.0), path);
    }

    /// Regression (#213): `route_path`'s ground branch deliberately returns `None` once a ground
    /// aircraft has landed at its destination (nothing left ahead of it to cross). That must not
    /// turn into an empty polyline for the map/detail-popup endpoint — it should fall back to the
    /// aircraft's own position rather than silently drawing nothing.
    #[test]
    fn falls_back_to_the_aircraft_s_own_position_when_the_route_has_collapsed() {
        assert_eq!(
            route_display_points(None, 38.95, -77.46),
            vec![[38.95, -77.46]]
        );
    }
}

/// Regression (#213): the actual `build_candidates`/`fca_counts` wiring around `prefile_position`
/// must skip a prefile with an unresolvable departure — not silently fall back to a fabricated
/// position. `prefile_position_tests` above only covers the pure helper in isolation; these cover
/// its use at the real call site, which is what a reverted `.unwrap_or((0.0, 0.0))` would break.
#[cfg(test)]
mod prefile_skip_integration_tests {
    use std::collections::HashMap;

    use chrono::Utc;

    use super::{ReleaseMap, build_candidates};
    use crate::feed::{
        airspace::Boundaries,
        nav::NavData,
        runway_db::RunwayDb,
        trajectory::ProfileTable,
        vatsim::{FlightPlan, Prefile, VatsimData},
        winds::Winds,
    };
    use crate::models::FcaBody;

    fn fca_crossing_the_corridor() -> FcaBody {
        FcaBody {
            id: "test".into(),
            name: "test".into(),
            color: "#fff".into(),
            artcc: "ZDC".into(),
            // Crosses the JFK->DCA corridor between the real WHITE (40.0) and SIE (39.1) fixes.
            points: sqlx::types::Json(vec![[39.5, -75.6], [39.5, -74.0]]),
            dests: vec![],
            origins: vec![],
            fixes: vec![],
            scope: vec![],
            min_fl: None,
            max_fl: None,
            dir: "any".into(),
            mode: "rate".into(),
            rate: 30,
            mit: 0,
            enabled: true,
            manual_order: vec![],
            manual_seq: false,
            updated_at: Utc::now(),
            updated_by: None,
            event_id: None,
            event_status: None,
            auto_publish: false,
        }
    }

    /// A prefile whose departure ICAO isn't in the (tiny, test) airport cache, but whose arrival
    /// and real enroute fixes (RBV/WHITE/SIE, via the bundled nav db) resolve on their own — the
    /// exact shape `nav::build_anchors` produces ≥2 anchors for without ever needing the departure.
    fn unresolvable_departure_prefile() -> VatsimData {
        VatsimData {
            prefiles: vec![Prefile {
                callsign: "TEST1".into(),
                flight_plan: Some(FlightPlan {
                    departure: "KJFK".into(), // deliberately absent from `airports` below
                    arrival: "KDCA".into(),
                    route: "RBV WHITE SIE".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn build_candidates_skips_a_prefile_whose_departure_does_not_resolve() {
        let nav = NavData::load();
        let airports: HashMap<String, (f64, f64)> =
            HashMap::from([("KDCA".to_string(), (38.85, -77.04))]); // no KJFK entry
        let fca = fca_crossing_the_corridor();
        let data = unresolvable_departure_prefile();
        let (flights, metas) = build_candidates(
            &fca,
            &data,
            &airports,
            &nav,
            &Boundaries::default(),
            &Winds::default(),
            &ProfileTable::default(),
            &ReleaseMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            Utc::now(),
            false,
        );
        assert!(
            flights.is_empty() && metas.is_empty(),
            "a prefile whose departure can't be resolved must be skipped, not counted via a \
             fabricated (0.0, 0.0) position: got {flights:?}"
        );
    }
}
