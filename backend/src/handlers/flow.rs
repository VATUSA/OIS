//! Flow handlers — FCA CRUD + a lightweight live-traffic feed for the FCA map.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
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
        airports::{Airport, AirportDb, field_elevation_ft},
        airspace::Boundaries,
        facilities, fca, flow as feed_flow,
        nav::NavData,
        nav_source, predict,
        runway_db::RunwayDb,
        taxi_estimate, trajectory,
        vatsim::FlightPlan,
        vatsim::VatsimData,
        winds::Winds,
    },
    jobs,
    models::{
        AircraftRoute, AirportGateBody, DataStatus, FcaBody, FcaFlight, FixPrediction,
        FixValidationBody, FlightAdvisory, FlightFcaCrossing, FlightGdp, FlightGroundStop,
        FlightProgram, IdstFlight, IdstResponse, ReleaseRequest, ReorderRequest,
        ResolveRouteRequest, ResolvedRoute, RouteBody, RouteWaypoint, TrafficAircraft,
        UpsertFcaRequest, UpsertRouteRequest,
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
    airports
        .get(&dep.to_ascii_uppercase())
        .map(|a| (a.lat, a.lon))
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

/// A snapshot of the manually excluded ("bogus") callsigns, keyed by ARTCC (#342) — read straight
/// from `AppState::flight_exclusions`, which the refresh job and the write handler keep current.
pub(crate) type ExclusionSet = std::collections::HashMap<String, std::collections::HashSet<String>>;

/// True when this callsign has been manually dropped as bogus (#342).
///
/// `artcc` scopes the check to one facility's removals — that is how the FCA surfaces (crossings,
/// metering, counts) filter, since an FCA belongs to an ARTCC. `None` means "excluded by **any**
/// facility", which is what the global traffic endpoints use: they carry no facility context, and a
/// flight with garbage data is garbage on every scope, so hiding it everywhere is what makes the
/// removal actually clear the map for every viewer.
///
/// Callsigns are stored upper-cased by the write handler; callers pass the raw feed callsign, which
/// VATSIM already emits upper-case.
pub(crate) fn is_manually_excluded(
    exclusions: &ExclusionSet,
    artcc: Option<&str>,
    callsign: &str,
) -> bool {
    match artcc {
        Some(a) => exclusions.get(a).is_some_and(|set| set.contains(callsign)),
        None => exclusions.values().any(|set| set.contains(callsign)),
    }
}

/// Every manually excluded callsign, across all facilities (#342) — the flat form
/// `feed::flow::compute` takes, since airport flow has no single facility context.
pub(crate) fn all_excluded_callsigns(
    exclusions: &ExclusionSet,
) -> std::collections::HashSet<String> {
    exclusions.values().flatten().cloned().collect()
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

/// **The** decision of whether a flight is in an FCA, and where it crosses (#360).
///
/// Four surfaces have to agree on this: the metering board (`build_candidates`, both its live-pilot
/// and prefile loops), the per-FCA badge counts (`fca_counts`), and the per-flight advisory
/// (`build_flight_advisory`). They used to restate the decision — manual exclusion, then
/// [`passes_filters`], then [`fca::crosses`], then [`passes_scope`] — in their own shape, so a new
/// condition had to be added in three places by hand and a miss showed up as the board, the badge
/// and the advisory quietly disagreeing about the same aircraft. #342 added its callsign check to
/// all of them one at a time, which is what prompted this.
///
/// Returns the crossing when the flight is included, so the one caller that needs the geometry
/// (`build_candidates`, for `along_nm` and the crossing position) gets it from the same call that
/// decides inclusion, while the count and advisory paths just ask `.is_some()`.
///
/// `path` stays a parameter: how each caller resolves the route legitimately differs (live position,
/// prefile departure field, or a route already resolved once for a cheap pre-filter), and that is
/// not part of the inclusion rule.
fn fca_crossing_for(
    fca: &FcaBody,
    airspace: &Boundaries,
    exclusions: &ExclusionSet,
    callsign: &str,
    fp: &FlightPlan,
    cur_alt_ft: Option<i64>,
    path: &[[f64; 2]],
) -> Option<fca::FcaCrossing> {
    if is_manually_excluded(exclusions, Some(&fca.artcc), callsign) {
        return None;
    }
    if !passes_filters(fca, fp, cur_alt_ft) {
        return None;
    }
    let cross = fca::crosses(path, &fca.points.0)?;
    passes_scope(fca, airspace, cross.lat, cross.lon).then_some(cross)
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

/// Protective margin added to the separation when **issuing** a release (#356).
///
/// A release is a commitment made now against *predicted* crossing times, so it should not take a
/// slot that only just fits — residual ETA drift then squeezes it. ~1.2 nm of extra in-trail at a
/// 280 kt crossing speed. Applied only on the release path: the ladder's own metering is unchanged,
/// so the crossing times controllers see don't shift.
const RELEASE_MARGIN_MS: i64 = 15_000;

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
    let exclusions = state.flight_exclusions.load_full();

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
        let exclusions = exclusions.as_ref();
        let mut tally =
            |callsign: &str, fp: &FlightPlan, lat: f64, lon: f64, hdg: i64, gs: i64, alt: i64| {
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
                    // Same inclusion decision the metering board makes, so a badge count can never
                    // disagree with the board it labels (#360).
                    if fca_crossing_for(f, airspace, exclusions, callsign, fp, Some(alt), &path)
                        .is_some()
                        && let Some(c) = counts.get_mut(&f.id)
                    {
                        *c += 1;
                    }
                }
            };

        for p in &snap.data.pilots {
            if let Some(fp) = &p.flight_plan {
                tally(
                    &p.callsign,
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
                tally(&pf.callsign, fp, lat, lon, 0, 0, 0);
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
        let fixes = fix_predictions(
            &state,
            nav,
            airports,
            fp,
            &points,
            Some((p.latitude, p.longitude, p.heading, p.groundspeed)),
            p.latitude,
            p.longitude,
            p.heading,
            p.groundspeed,
            p.altitude as f64,
            Utc::now(),
        );
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
            fixes,
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
        let fixes = prefile_fix_predictions(&state, nav, airports, fp, Utc::now());
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
            fixes,
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
    let current = nav_source::current_cycle();
    DataStatus {
        nav_cycle: nav.cycle().to_string(),
        nav_cycle_current: current.format("%Y-%m-%d").to_string(),
        nav_cycles_behind: nav_source::cycles_behind(nav.cycle(), current),
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
            // Same inclusion decision as the metering board and the badge counts (#360). If it
            // doesn't cross, metering this FCA can't produce a slot for it.
            let crosses = match (&path, &fp) {
                (Some(p), Some(plan)) => fca_crossing_for(
                    &fca,
                    state.airspace.as_ref(),
                    state.flight_exclusions.load().as_ref(),
                    &cs,
                    plan,
                    Some(altitude),
                    p,
                )
                .is_some(),
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

/// Holds the single-flight claim on a manual data refresh and releases it on drop, so a cancelled
/// request (client disconnect), an early return or a panic can't latch the flag and lock the
/// endpoint out for the rest of the process's life.
struct DataRefreshClaim<'a>(&'a AtomicBool);

impl<'a> DataRefreshClaim<'a> {
    /// Claim the refresh slot, or `None` when one is already running.
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for DataRefreshClaim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Force an immediate nav + winds refresh, then return the updated status. Failures are
/// logged and leave the current data in place — the response still carries the resulting status, so
/// the caller compares it (cycle age, wind-station count) to tell a real refresh from a fallback.
///
/// One refresh at a time: a rebuild is a full upstream download and parse, and the control is global
/// to every flow controller, so a concurrent press gets a 409 instead of starting a second rebuild.
#[utoipa::path(
    post,
    path = "/api/v1/flow/data-refresh",
    tag = "flow",
    responses((status = 200, body = DataStatus), (status = 401), (status = 409))
)]
pub async fn data_refresh(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
) -> Result<Json<DataStatus>, ApiError> {
    let Some(_claim) = DataRefreshClaim::acquire(&state.data_refresh_in_flight) else {
        tracing::info!("manual data refresh rejected: one is already running");
        return Err(ApiError::Conflict);
    };
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
    Ok(Json(build_data_status(&state)))
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/traffic",
    tag = "flow",
    responses((status = 200, body = Vec<TrafficAircraft>), (status = 401))
)]
pub async fn list_traffic(State(state): State<AppState>) -> Json<Vec<TrafficAircraft>> {
    let snapshot = state.feed.read().await.snapshot.clone();
    let exclusions = state.flight_exclusions.load_full();
    let aircraft = snapshot
        .as_ref()
        .map(|snap| traffic_from(&snap.data, exclusions.as_ref()))
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
    let exclusions = state.flight_exclusions.load_full();
    let aircraft = tokio::task::spawn_blocking(move || {
        project_traffic(
            &snap.data,
            nav.as_ref(),
            airports.as_ref(),
            profiles.as_ref(),
            winds.as_ref(),
            exclusions.as_ref(),
            q.offset_sec,
        )
    })
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(Json(aircraft))
}

/// Map a snapshot's pilots to the lightweight map-traffic shape (drops position-less aircraft).
/// Pure of the live feed so the historical replay can reuse it against a reconstructed snapshot.
pub(crate) fn traffic_from(data: &VatsimData, exclusions: &ExclusionSet) -> Vec<TrafficAircraft> {
    data.pilots
        .iter()
        .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
        // Manually dropped as bogus (#342). This endpoint carries no facility context, so a
        // callsign any facility removed is hidden — which is what clears it off the map for
        // every viewer.
        .filter(|p| !is_manually_excluded(exclusions, None, &p.callsign))
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
    exclusions: &ExclusionSet,
    offset_sec: i64,
) -> Vec<TrafficAircraft> {
    let offset_sec = offset_sec as f64;
    data.pilots
        .iter()
        .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
        .filter(|p| !is_manually_excluded(exclusions, None, &p.callsign))
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
                field_elevation_ft(airports, &fp.arrival),
                cruise_ft,
                cruise_tas,
                profile,
                headwind,
            )
            .anchor_to_observed_gs(p.groundspeed as f64);
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

/// The route-length scale factor for the debug per-fix table (#225 rework): `path_len_ge_2`
/// anchors — `> 2` means the nav engine placed at least one real enroute point; exactly `2` (a
/// direct/unexpandable filed route) is no better than a great circle. Mirrors
/// `predict::arrival_eta`'s own `!airborne && !enroute_resolved` padding exactly, so a
/// ground/prefile aircraft on an unresolved route gets the same [`predict::GROUND_ROUTE_FACTOR`]
/// this debug table's ETAs would otherwise silently disagree with the real metering/ladder model
/// on (a live-confirmed ~12% undercount before this fix).
fn ground_route_scale(airborne: bool, path_len: usize) -> f64 {
    let enroute_resolved = path_len > 2;
    if !airborne && !enroute_resolved {
        predict::GROUND_ROUTE_FACTOR
    } else {
        1.0
    }
}

/// Per-fix predictions for the debug-mode route breakdown (#225): the same trajectory/ETA model
/// FCA metering, the arrival ladder, and runway ETE all resolve through
/// (`feed::predict`/`feed::trajectory`), just queried at every named fix instead of one crossing
/// point. `lat`/`lon`/`hdg`/`gs` are the aircraft's live state, or the departure airport with
/// `hdg`/`gs` `0` for a prefile (see [`prefile_fix_predictions`]). `pilot_pos` is the aircraft's real
/// position for the ground-allowance gate/runway match — `Some` for any connected pilot (even
/// parked, `gs`/`hdg` both `0`), `None` only for an actual prefile — matching
/// [`feed_flow::ground_estimate`]'s own `pilot_pos` convention; it must not be re-derived from
/// `gs`/`hdg` being nonzero, or a pilot idling at the gate is treated as position-less. `path` is
/// the caller's already-resolved `fca::route_path` polyline for the same aircraft (reused for the
/// headwind sample rather than re-resolving the route a second time). Returns an empty list when
/// the route can't be resolved, exactly like `points`/`waypoints` already tolerate.
#[allow(clippy::too_many_arguments)]
fn fix_predictions(
    state: &AppState,
    nav: &NavData,
    airports: &AirportDb,
    fp: &FlightPlan,
    path: &[[f64; 2]],
    pilot_pos: Option<(f64, f64, i64, i64)>,
    lat: f64,
    lon: f64,
    hdg: i64,
    gs: i64,
    cur_alt_ft: f64,
    now: DateTime<Utc>,
) -> Vec<FixPrediction> {
    if path.len() < 2 {
        return Vec::new();
    }
    let Some(named) = fca::route_path_named(
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
        return Vec::new();
    };

    let airborne = gs >= 50;
    let (ty, wake) = fp.aircraft_type_wake();
    let profiles = state.aircraft_profiles.load_full();
    let profile = profiles.resolve(&ty, &wake);
    let cruise_alt = trajectory::parse_alt_ft(&fp.altitude);
    let filed_tas: f64 = fp.cruise_tas.parse().unwrap_or(0.0);
    let cruise_tas = trajectory::capped_cruise_tas(filed_tas, cruise_alt, profile);
    let headwind = state.winds.load_full().route_headwind(path, cruise_alt);
    let route_scale = ground_route_scale(airborne, path.len());
    let route_len = predict::path_len_nm(path) * route_scale;

    let dep = fp.departure.to_ascii_uppercase();
    let ground_allowance = if airborne {
        0.0
    } else {
        let aircraft = (!fp.aircraft_short.is_empty()).then_some(fp.aircraft_short.as_str());
        feed_flow::resolve_ground_allowance_sec(
            &state.gates.load_full(),
            &state.runways,
            &state.taxi_estimate_samples.load_full(),
            &dep,
            aircraft,
            pilot_pos,
        )
    };

    let start_alt = if airborne { cur_alt_ft } else { 0.0 };
    let vp = trajectory::VerticalProfile::build(
        start_alt,
        route_len,
        field_elevation_ft(airports, &fp.arrival),
        cruise_alt,
        cruise_tas,
        profile,
        headwind,
    );
    let vp = if airborne {
        vp.anchor_to_observed_gs(gs as f64)
    } else {
        vp
    };

    let arr_ll = airports.get(&fp.arrival.to_ascii_uppercase()).map(
        |&Airport {
             lat: la, lon: lo, ..
         }| [la, lo],
    );
    let mut out = Vec::with_capacity(named.len());
    for (i, (name, flat, flon, along_nm)) in named.iter().enumerate() {
        let (flat, flon, along_nm) = (*flat, *flon, *along_nm * route_scale);
        let target_d = (route_len - along_nm).max(0.0);
        let mut sec = vp.time_between(route_len, target_d);
        if !airborne {
            sec += ground_allowance;
        }
        let next = named.get(i + 1).map(|(_, la, lo, _)| [*la, *lo]);
        // A degenerate target (this fix already *is* the arrival airport, e.g. the last named
        // point on the route) would give a meaningless 0° bearing — fall back to current heading.
        let heading_deg = next
            .or(arr_ll)
            .filter(|&to| feed_flow::gc_dist(flat, flon, to[0], to[1]) > 0.1)
            .map(|to| fca::bearing_deg([flat, flon], to).round() as i64)
            .unwrap_or(hdg);

        out.push(FixPrediction {
            name: name.clone(),
            lat: flat,
            lon: flon,
            eta: now + Duration::seconds(sec as i64),
            altitude_ft: vp.alt_at(target_d).round() as i64,
            groundspeed_kt: vp.ground_speed_at(target_d).round() as i64,
            heading_deg,
            distance_nm: along_nm.round() as i64,
        });
    }
    out
}

/// [`fix_predictions`] for a prefile: timed from its departure airport, exactly like
/// `feed_flow::ground_estimate` and every other prefile caller (`prefile_position`, #213) — never a
/// `(0.0, 0.0)` placeholder, which `route_path`'s ground trimming would project onto the route and
/// prepend (thousands of nm of phantom distance, or a route collapsed to nothing). A prefile whose
/// departure doesn't resolve gets no per-fix table, the same skip the other prefile callers apply.
fn prefile_fix_predictions(
    state: &AppState,
    nav: &NavData,
    airports: &AirportDb,
    fp: &FlightPlan,
    now: DateTime<Utc>,
) -> Vec<FixPrediction> {
    let Some((lat, lon)) = prefile_position(airports, &fp.departure) else {
        return Vec::new();
    };
    // The *full* anchor set (named + unnamed), not the named-only `points` track, so the route
    // length / headwind sample isn't undercounted by a skipped unnamed procedure-leg point.
    let path = fca::route_path(
        nav,
        airports,
        &fp.departure,
        &fp.arrival,
        &fp.route,
        lat,
        lon,
        0,
        0,
    )
    .unwrap_or_default();
    fix_predictions(
        state, nav, airports, fp, &path, None, lat, lon, 0, 0, 0.0, now,
    )
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
    cross_speed: f64,
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
        cross_speed: cross_speed.round() as i64,
        cruise_alt: cruise_alt.round() as i64,
        headwind: headwind.map(|h| h.round() as i64),
        unresolved,
        taxi_estimate: taxi.map(|t| crate::models::TaxiEstimateDebug {
            gate: t.gate_id.clone(),
            runway: t.runway.clone(),
            pushback_sec: t.pushback.value_sec.round() as i64,
            pushback_tier: t.pushback.tier.label().to_string(),
            pushback_samples: t.pushback.sample_count as i64,
            startup_sec: t.startup.value_sec.round() as i64,
            startup_tier: t.startup.tier.label().to_string(),
            startup_samples: t.startup.sample_count as i64,
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
    exclusions: &ExclusionSet,
    now: DateTime<Utc>,
    debug: bool,
) -> (Vec<FcaFlight>, Vec<fca::MeterInput>) {
    let mut flights = Vec::new();
    let mut metas = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let airborne = p.groundspeed >= 50;
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
        // One decision, shared with the badge counts and the advisory (#360). Skipping here is
        // before either push, which matters: `metas` and `flights` are positionally coupled and
        // `finalize` consumes them as parallel slices.
        let Some(cross) = fca_crossing_for(
            fca,
            airspace,
            exclusions,
            &p.callsign,
            fp,
            Some(p.altitude),
            &path,
        ) else {
            continue;
        };
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
        let pred = predict::eta_along_route(
            airborne,
            route_len,
            cross.along_nm,
            p.altitude as f64,
            p.groundspeed as f64,
            cruise,
            cruise_tas,
            field_elevation_ft(airports, &fp.arrival),
            profile,
            headwind,
            allowance,
            now,
        );
        let eta = pred.eta;
        // MIT is a distance *at the crossing fix*, so the gap must be sized with the speed the
        // aircraft actually crosses at. Cruise groundspeed under-provisions every descending
        // arrival — a 20 MIT flow over a low fix realized ~12 nm (#355). Bound once so the metered
        // gap and the debug view that explains it can never disagree about the floor.
        let cross_speed = pred.gs_kt.max(120.0);
        let rel = releases.get(&p.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.timestamp_millis(),
            airborne,
            cross_speed,
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
                cross_speed,
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
        // Same shared decision as the live-pilot loop above (#360); a prefile has no live
        // altitude, so it qualifies on its filed cruise alone.
        let Some(cross) =
            fca_crossing_for(fca, airspace, exclusions, &pf.callsign, fp, None, &path)
        else {
            continue;
        };
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
        let pred = predict::eta_along_route(
            false,
            route_len,
            cross.along_nm,
            0.0,
            0.0,
            cruise,
            cruise_tas,
            field_elevation_ft(airports, &fp.arrival),
            profile,
            headwind,
            allowance,
            now,
        );
        let eta = pred.eta;
        // Crossing speed, not cruise — see the airborne/ground site above (#355).
        let cross_speed = pred.gs_kt.max(120.0);
        let rel = releases.get(&pf.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.timestamp_millis(),
            airborne: false,
            cross_speed,
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
                cross_speed,
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

/// The controller's manual crossing order as candidate indices, when the FCA is in manual sequence.
/// `None` means auto — `fca::meter` sequences by time itself.
///
/// Shared by [`finalize`] and `mark_release` so a release is metered against exactly the sequence the
/// ladder resolves, rather than a second, subtly different derivation of it (#356).
fn manual_order(fca: &FcaBody, flights: &[FcaFlight]) -> Option<Vec<usize>> {
    (fca.manual_seq && !fca.manual_order.is_empty()).then(|| {
        fca.manual_order
            .iter()
            .filter_map(|cs| flights.iter().position(|f| &f.callsign == cs))
            .collect()
    })
}

/// The crossing times a newly released aircraft must be spaced clear of: every **other** pinned
/// crossing — airborne, or an already-issued CFR — at its **metered** time, not its raw ETA.
///
/// The distinction is the whole of #356. `fca::meter` pushes an airborne arrival later than its ETA
/// whenever it still owes spacing to the aircraft ahead of it, so spacing a release against raw ETAs
/// hands the departure a gap that closes as soon as that arrival takes its own spacing. Unpinned
/// ground traffic is excluded: it has no committed time to conflict with and floats around the
/// release itself.
fn committed_crossings(
    metas: &[fca::MeterInput],
    metered: &[fca::MeterOutput],
    releasing: usize,
) -> Vec<i64> {
    metas
        .iter()
        .zip(metered)
        .enumerate()
        .filter(|(j, _)| *j != releasing)
        .filter(|(_, (m, _))| m.airborne || m.frozen_ms.is_some())
        .map(|(_, (_, out))| out.sched_ms)
        .collect()
}

/// The separation a crossing must hold at this FCA, in ms — MIT converted at the aircraft's own
/// crossing speed, or the flat rate interval.
fn separation_ms(fca: &FcaBody, cross_speed: f64) -> i64 {
    if fca.mode == "mit" {
        ((fca.mit as f64 / cross_speed.max(60.0)) * 3600.0 * 1000.0) as i64
    } else if fca.rate > 0 {
        (3600.0 / fca.rate as f64 * 1000.0) as i64
    } else {
        0
    }
}

/// The crossing time to pin when a controller marks an aircraft ready (#356).
///
/// This is the whole of the RDY decision — `mark_release` only chooses between it and an explicit
/// SET time. It is pure so that decision is testable without a pool, auth or a feed snapshot:
/// asserting `earliest_slot`'s arithmetic against hand-built arguments proves nothing about what
/// the handler actually does.
///
/// **Auto:** the earliest slot clear of every other pinned crossing at its **metered** time, not
/// its raw ETA — an airborne arrival that still owes spacing to the aircraft ahead of it is metered
/// later, so a gap measured at its ETA closes under the departure we just released.
///
/// **Manual:** the controller's order *is* the sequence, so the release takes the slot the ladder
/// already assigned it. Re-running `earliest_slot` here would space it against crossings it pushed
/// later **itself** — in manual mode `meter` chains every aircraft, so the releasing ground
/// aircraft moves everything behind it — which lands the release behind an aircraft the controller
/// explicitly ordered it ahead of, and does not even converge: re-metering moves them again.
///
/// Both carry [`RELEASE_MARGIN_MS`] on top, so a release never takes a slot that only just fits.
fn rdy_slot(
    fca: &FcaBody,
    metas: &[fca::MeterInput],
    metered: &[fca::MeterOutput],
    ti: usize,
    manual: bool,
) -> i64 {
    let sep_ms = separation_ms(fca, metas[ti].cross_speed);
    if manual {
        return metered[ti].sched_ms + RELEASE_MARGIN_MS;
    }
    let committed = committed_crossings(metas, metered, ti);
    fca::earliest_slot(metas[ti].eta_ms, &committed, sep_ms + RELEASE_MARGIN_MS)
}

/// Meter the candidates (auto, or the FCA's manual order) and finalize sequence/delay.
fn finalize(
    fca: &FcaBody,
    mut flights: Vec<FcaFlight>,
    metas: &[fca::MeterInput],
) -> Vec<FcaFlight> {
    let order = manual_order(fca, &flights);
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
    let flight_exclusions = state.flight_exclusions.load_full();
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
            flight_exclusions.as_ref(),
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
    let flight_exclusions = state.flight_exclusions.load_full();

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
                flight_exclusions.as_ref(),
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
            state.flight_exclusions.load_full().as_ref(),
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

    let order = manual_order(&fca, &flights);
    let metered = fca::meter(&metas, &fca.mode, fca.rate, fca.mit, order.as_deref());

    let cta = match payload.ready.as_deref().filter(|s| !s.trim().is_empty()) {
        // SET: pin the crossing so wheels-up lands on the requested time.
        Some(ready) => parse_hhmm_z(ready, now).ok_or(ApiError::BadRequest)? + (eta_ms - now_ms),
        // RDY: the metered slot. Every part of that decision lives in `rdy_slot`.
        None => rdy_slot(&fca, &metas, &metered, ti, order.is_some()),
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
            state.flight_exclusions.load_full().as_ref(),
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
    use crate::feed::airports::Airport;
    use std::collections::HashMap;

    use super::{VatsimData, project_traffic};
    use crate::feed::{
        nav::NavData, trajectory::ProfileTable, vatsim::FlightPlan, vatsim::Pilot, vatsim::Prefile,
        winds::Winds,
    };

    fn airports() -> crate::feed::airports::AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
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
            &HashMap::new(),
            0,
        );
        assert_eq!(out.len(), 1);
        assert!((out[0].lat - 40.2).abs() < 1e-9);
        assert!((out[0].lon - -74.0).abs() < 1e-9);
    }

    /// Regression (#342): the exclusion must be applied *at the call site*, not merely available.
    /// Every other test here passes an **empty** exclusion map, so neutralising the filter in
    /// `project_traffic` / `traffic_from` changes nothing they assert — the whole feature could be
    /// removed from production with the suite still green. These drive a **populated** set.
    #[test]
    fn a_manually_excluded_callsign_is_dropped_from_projected_traffic() {
        let data = VatsimData {
            pilots: vec![airborne_pilot()],
            ..Default::default()
        };
        let excluded: super::ExclusionSet = HashMap::from([(
            "ZDC".to_string(),
            std::collections::HashSet::from(["TEST1".to_string()]),
        )]);

        let kept = project_traffic(
            &data,
            &NavData::load(),
            &airports(),
            &ProfileTable::default(),
            &Winds::default(),
            &HashMap::new(),
            0,
        );
        assert_eq!(
            kept.len(),
            1,
            "sanity: the aircraft is there when nothing is excluded"
        );

        let dropped = project_traffic(
            &data,
            &NavData::load(),
            &airports(),
            &ProfileTable::default(),
            &Winds::default(),
            &excluded,
            0,
        );
        assert!(
            dropped.is_empty(),
            "a manually excluded callsign must not appear in projected traffic, got {:?}",
            dropped.iter().map(|a| &a.callsign).collect::<Vec<_>>()
        );
    }

    /// The live-traffic sibling of the above, and the endpoint the controller actually watches
    /// clear. Also pins the cross-facility semantics: this surface carries no facility context, so
    /// *any* facility's removal hides the aircraft for everyone.
    #[test]
    fn a_manually_excluded_callsign_is_dropped_from_live_traffic() {
        let data = VatsimData {
            pilots: vec![airborne_pilot()],
            ..Default::default()
        };
        assert_eq!(super::traffic_from(&data, &HashMap::new()).len(), 1);

        let excluded_elsewhere: super::ExclusionSet = HashMap::from([(
            "ZNY".to_string(),
            std::collections::HashSet::from(["TEST1".to_string()]),
        )]);
        assert!(
            super::traffic_from(&data, &excluded_elsewhere).is_empty(),
            "the global traffic surface hides a callsign any facility removed"
        );
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
            &HashMap::new(),
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
            &HashMap::new(),
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
            &HashMap::new(),
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
            &HashMap::new(),
            600,
        );
        assert!(out.is_empty());
    }
}

#[cfg(test)]
mod prefile_position_tests {
    use crate::feed::airports::Airport;
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
            HashMap::from([("KJFK".to_string(), Airport::at(40.64, -73.78))]);
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
    use crate::feed::airports::Airport;
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
    /// The same shape, but with a departure the tiny test airport cache *does* resolve, so the
    /// prefile is a real candidate — the baseline the #342 exclusion test filters against.
    fn unresolvable_departure_prefile_with_resolvable_departure() -> VatsimData {
        VatsimData {
            prefiles: vec![Prefile {
                callsign: "TEST1".into(),
                flight_plan: Some(FlightPlan {
                    departure: "KJFK".into(),
                    arrival: "KDCA".into(),
                    route: "RBV WHITE SIE".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

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

    /// Regression (#342): the FCA crossing list must consult the exclusion set too. Scoped by the
    /// FCA's own ARTCC here (unlike the global traffic surfaces), so a removal by a *different*
    /// facility must leave the flight on this FCA's board.
    #[test]
    fn build_candidates_drops_a_manually_excluded_prefile_for_its_own_artcc() {
        let nav = NavData::load();
        let airports: crate::feed::airports::AirportDb = HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        let fca = fca_crossing_the_corridor(); // artcc: "ZDC"
        let data = unresolvable_departure_prefile_with_resolvable_departure();

        let candidates = |ex: &super::ExclusionSet| {
            let (flights, _) = build_candidates(
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
                ex,
                Utc::now(),
                false,
            );
            flights.len()
        };

        assert_eq!(
            candidates(&HashMap::new()),
            1,
            "sanity: TEST1 crosses this FCA"
        );

        let other_facility: super::ExclusionSet = HashMap::from([(
            "ZNY".to_string(),
            std::collections::HashSet::from(["TEST1".to_string()]),
        )]);
        assert_eq!(
            candidates(&other_facility),
            1,
            "another facility's removal must not clear this ZDC FCA's board"
        );

        let own_facility: super::ExclusionSet = HashMap::from([(
            "ZDC".to_string(),
            std::collections::HashSet::from(["TEST1".to_string()]),
        )]);
        assert_eq!(
            candidates(&own_facility),
            0,
            "the owning facility's removal must drop the flight from the crossing list"
        );
    }

    #[test]
    fn build_candidates_skips_a_prefile_whose_departure_does_not_resolve() {
        let nav = NavData::load();
        let airports: crate::feed::airports::AirportDb =
            HashMap::from([("KDCA".to_string(), Airport::at(38.85, -77.04))]); // no KJFK entry
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

#[cfg(test)]
mod ground_route_scale_tests {
    use super::ground_route_scale;

    /// Regression (#225 rework): `fix_predictions` originally fed `path_len_nm` straight into the
    /// debug table's ETA math with no padding, so a ground/prefile aircraft on an unresolved
    /// (direct-filed, 2-anchor) route showed ETAs ~12% earlier than `predict::arrival_eta` — the
    /// real model backing metering/the ladder/runway ETE — would compute for the identical flight.
    #[test]
    fn pads_a_ground_aircraft_on_an_unresolved_two_anchor_route() {
        assert_eq!(
            ground_route_scale(false, 2),
            crate::feed::predict::GROUND_ROUTE_FACTOR
        );
    }

    #[test]
    fn does_not_pad_a_ground_aircraft_once_the_route_has_a_real_enroute_point() {
        assert_eq!(ground_route_scale(false, 3), 1.0);
    }

    #[test]
    fn never_pads_an_airborne_aircraft_even_on_an_unresolved_route() {
        // An airborne aircraft is timed against the route the map actually draws (matching
        // `predict::arrival_eta`'s own airborne branch) — padding it would double-correct.
        assert_eq!(ground_route_scale(true, 2), 1.0);
    }
}

#[cfg(test)]
mod prefile_fix_predictions_tests {
    use crate::feed::airports::Airport;
    use std::collections::HashMap;

    use chrono::{DateTime, Utc};

    use super::prefile_fix_predictions;
    use crate::feed::{
        flow as feed_flow, nav::NavData, predict, runway_db::RunwayDb, trajectory::AircraftProfile,
        vatsim::FlightPlan, winds::Winds,
    };
    use crate::scope_test_support::test_state;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn state() -> crate::state::AppState {
        // Lazy pool: never connects — `prefile_fix_predictions` reads only the in-memory caches.
        let pool = sqlx::PgPool::connect_lazy("postgres://unused@127.0.0.1/unused").unwrap();
        test_state(pool, HashMap::new())
    }

    fn plan(dep: &str, arr: &str, route: &str) -> FlightPlan {
        FlightPlan {
            departure: dep.into(),
            arrival: arr.into(),
            route: route.into(),
            altitude: "FL350".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        }
    }

    fn airports() -> crate::feed::airports::AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
            ("KIAD".to_string(), Airport::at(38.95, -77.46)),
        ])
    }

    /// The same prefile through the real ETA model (`ground_estimate`'s `arrival_eta` call, from
    /// the departure airport) — what metering / airport-flow demand show for it.
    fn real_arrival(
        nav: &NavData,
        ap: &crate::feed::airports::AirportDb,
        fp: &FlightPlan,
    ) -> predict::ArrivalPrediction {
        let dep_ll = ap[&fp.departure];
        let arr_ll = ap[&fp.arrival];
        let profile = AircraftProfile::default();
        let cruise_ft = crate::feed::trajectory::parse_alt_ft(&fp.altitude);
        let allowance = feed_flow::resolve_ground_allowance_sec(
            &HashMap::new(),
            &RunwayDb::load(),
            &HashMap::new(),
            &fp.departure,
            None,
            None,
        );
        predict::arrival_eta(
            nav,
            ap,
            &Winds::default(),
            &profile,
            &predict::ArrivalInput {
                dep: &fp.departure,
                arr: &fp.arrival,
                route: &fp.route,
                pos: [dep_ll.lat, dep_ll.lon],
                alt_ft: 0.0,
                gs: 0,
                hdg: 0,
                arr_ll: [arr_ll.lat, arr_ll.lon],
                cruise_ft,
                cruise_tas: crate::feed::trajectory::capped_cruise_tas(440.0, cruise_ft, &profile),
            },
            allowance,
            now(),
        )
    }

    /// Regression (#225 QA): the prefile table was timed from a `(0.0, 0.0)` placeholder, which
    /// `route_path`'s ground trimming prepends — KJFK→KDCA read ~4,900 nm instead of ~224 nm.
    /// The last fix (the arrival field) must carry the real model's distance and ETA.
    #[tokio::test]
    async fn a_prefile_is_timed_from_its_departure_and_matches_the_real_arrival_eta() {
        let (st, nav, ap) = (state(), NavData::load(), airports());
        let fp = plan("KJFK", "KDCA", "RBV WHITE SIE");
        let fixes = prefile_fix_predictions(&st, &nav, &ap, &fp, now());
        let real = real_arrival(&nav, &ap, &fp);

        let last = fixes.last().expect("the resolved route yields fixes");
        assert_eq!(last.name, "KDCA");
        assert!(
            (last.distance_nm as f64 - real.route_nm).abs() <= 1.0,
            "last fix {} nm vs real model {:.1} nm",
            last.distance_nm,
            real.route_nm
        );
        assert!(
            (last.eta - real.eta).num_seconds().abs() <= 1,
            "last fix ETA {} vs real model {}",
            last.eta,
            real.eta
        );
        assert!(fixes.iter().any(|f| f.name == "RBV"));
    }

    /// Regression (#225 QA): on a reversed, unresolved (direct) KIAD→KJFK route `(0,0)` projects
    /// onto the *arrival* end and collapsed the table to nothing; from the departure it's a 2-anchor
    /// route, so the ground `GROUND_ROUTE_FACTOR` padding must apply to the fix distance too.
    #[tokio::test]
    async fn a_direct_prefile_keeps_its_table_and_pads_the_distance_like_the_real_model() {
        let (st, nav, ap) = (state(), NavData::load(), airports());
        let fp = plan("KIAD", "KJFK", "");
        let fixes = prefile_fix_predictions(&st, &nav, &ap, &fp, now());
        let real = real_arrival(&nav, &ap, &fp);

        let last = fixes
            .last()
            .expect("a direct prefile still yields the arrival fix");
        assert_eq!(last.name, "KJFK");
        assert!(
            (last.distance_nm as f64 - real.route_nm).abs() <= 1.0,
            "last fix {} nm vs real (padded) model {:.1} nm",
            last.distance_nm,
            real.route_nm
        );
        assert!((last.eta - real.eta).num_seconds().abs() <= 1);
    }

    #[tokio::test]
    async fn a_prefile_whose_departure_does_not_resolve_gets_no_table() {
        let (st, nav) = (state(), NavData::load());
        let ap = HashMap::from([("KDCA".to_string(), Airport::at(38.85, -77.04))]); // no KJFK
        let fp = plan("KJFK", "KDCA", "RBV WHITE SIE");
        assert!(prefile_fix_predictions(&st, &nav, &ap, &fp, now()).is_empty());
    }
}

#[cfg(test)]
mod data_status_tests {
    use std::sync::Arc;

    use chrono::Duration;
    use serde_json::json;

    use super::build_data_status;
    use crate::{
        feed::{nav::NavData, nav_source},
        state::AppState,
    };

    fn status_for_cycle(cycle: &str) -> crate::models::DataStatus {
        let state = AppState::without_db();
        let meta = json!({ "nasrCycleDate": cycle }).to_string();
        state.nav.store(Arc::new(NavData::from_json(
            "{}", "{}", "{}", "{}", "{}", &meta, "{}",
        )));
        build_data_status(&state)
    }

    #[test]
    fn reports_how_many_cycles_the_loaded_nav_data_trails_current() {
        let current = nav_source::current_cycle();
        let two_behind = (current - Duration::days(56))
            .format("%Y-%m-%d")
            .to_string();
        let status = status_for_cycle(&two_behind);
        assert_eq!(status.nav_cycle, two_behind);
        assert_eq!(
            status.nav_cycle_current,
            current.format("%Y-%m-%d").to_string()
        );
        assert_eq!(status.nav_cycles_behind, Some(2));
        assert_eq!(status_for_cycle("unknown").nav_cycles_behind, None);
    }
}

#[cfg(test)]
mod data_refresh_claim_tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::DataRefreshClaim;

    #[test]
    fn a_second_claim_is_refused_while_the_first_is_held() {
        let flag = AtomicBool::new(false);
        let first = DataRefreshClaim::acquire(&flag).expect("first claim");
        assert!(
            DataRefreshClaim::acquire(&flag).is_none(),
            "a concurrent refresh must be refused, not started alongside the running one"
        );
        drop(first);
    }

    /// The flag must clear on drop, not on the happy path: an early return, a client disconnect that
    /// cancels the future, or a panic inside the fetch would otherwise latch it and wedge the
    /// endpoint at 409 for the rest of the process's life.
    #[test]
    fn dropping_the_claim_releases_the_slot() {
        let flag = AtomicBool::new(false);
        drop(DataRefreshClaim::acquire(&flag).expect("first claim"));
        assert!(!flag.load(Ordering::Acquire), "drop must clear the flag");
        assert!(
            DataRefreshClaim::acquire(&flag).is_some(),
            "a refresh must be possible again once the previous one finished"
        );
    }

    #[test]
    fn a_claim_is_refused_when_the_flag_is_already_set() {
        let flag = AtomicBool::new(true);
        assert!(DataRefreshClaim::acquire(&flag).is_none());
    }
}

#[cfg(test)]
mod release_spacing_tests {
    use super::{RELEASE_MARGIN_MS, committed_crossings, rdy_slot, separation_ms};
    use crate::feed::fca::{self, MeterInput};
    use crate::models::FcaBody;
    use chrono::Utc;

    const SEP_MS: i64 = 120_000; // 30/hr

    fn cand(eta_ms: i64, airborne: bool, frozen_ms: Option<i64>) -> MeterInput {
        MeterInput {
            eta_ms,
            airborne,
            cross_speed: 300.0,
            frozen_ms,
        }
    }

    fn rate_fca(manual_order: Vec<String>) -> FcaBody {
        FcaBody {
            id: "t".into(),
            name: "t".into(),
            color: "#fff".into(),
            artcc: "ZDC".into(),
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
            manual_seq: !manual_order.is_empty(),
            manual_order,
            updated_at: Utc::now(),
            updated_by: None,
            event_id: None,
            event_status: None,
            auto_publish: false,
        }
    }

    /// Leader arrival at 0s; arrival A's ETA is 60s but it owes the leader its spacing, so the
    /// ladder meters A to 120s; a ground departure is released behind them.
    fn leader_arrival_and_departure() -> Vec<MeterInput> {
        vec![
            cand(0, true, None),
            cand(60_000, true, None),
            cand(180_000, false, None),
        ]
    }

    /// **#355 and #356 together** — the reconciliation this branch exists to prove.
    ///
    /// Both bugs under-space a released departure and they **compound**, because `rdy_slot` sizes
    /// its gap from `metas[ti].cross_speed` (#355's value) and measures it against the committed
    /// crossings (#356's set):
    ///
    /// - **#355** — the gap was `MIT ÷ cruise`, so it was too *short* for the speed the aircraft
    ///   actually crosses at.
    /// - **#356** — the gap was measured from arrivals' raw ETAs, so it started from the *wrong
    ///   place* and closed once those arrivals took their own spacing.
    ///
    /// Fixing either alone still leaves a squeezed release, which is why they were asked to be
    /// tested together. Here a 20 MIT flow over a low arrival fix (282.5 kt crossing, not the
    /// 440 kt filed cruise) shows both corrections live at once.
    #[test]
    fn the_crossing_speed_gap_and_the_metered_sta_baseline_compound() {
        const CROSSING_GS: f64 = 282.5; // what #355 feeds in at a low arrival fix
        const CRUISE_GS: f64 = 440.0; // what it used to feed in
        const MIT: i32 = 20;

        let mut fca = rate_fca(vec![]);
        fca.mode = "mit".into();
        fca.rate = 0;
        fca.mit = MIT;

        // #355: the gap is sized on the crossing speed, so it is materially longer than the
        // cruise-sized one it replaced.
        let gap = separation_ms(&fca, CROSSING_GS);
        let cruise_gap = separation_ms(&fca, CRUISE_GS);
        assert!(
            gap > cruise_gap,
            "#355: a crossing-speed gap ({gap} ms) must exceed the cruise-sized one ({cruise_gap} ms)"
        );
        // It is the gap that actually delivers the configured distance at that speed.
        let realized_nm = gap as f64 / 1000.0 / 3600.0 * CROSSING_GS;
        assert!(
            (realized_nm - MIT as f64).abs() < 0.5,
            "#355: the gap must realize the configured {MIT} MIT, got {realized_nm:.1} nm"
        );

        // #356: and that gap is measured from the arrivals' *metered* crossings.
        let metas: Vec<MeterInput> = leader_arrival_and_departure()
            .into_iter()
            .map(|m| MeterInput {
                cross_speed: CROSSING_GS,
                ..m
            })
            .collect();
        let metered = fca::meter(&metas, &fca.mode, fca.rate, fca.mit, None);
        let a_sta = metered[1].sched_ms;
        assert!(
            a_sta > metas[1].eta_ms,
            "the scenario needs an arrival the ladder pushes back; got STA {a_sta} = ETA"
        );

        let cta = rdy_slot(&fca, &metas, &metered, 2, false);

        // The combined guarantee: a full crossing-speed-sized gap behind A's *real* crossing.
        assert!(
            cta - a_sta >= gap,
            "combined: the release must sit a full crossing-speed gap ({gap} ms) behind the \
             arrival's metered crossing, got {} ms",
            cta - a_sta
        );

        // Neither fix alone would have got here. Sizing on cruise (pre-#355) would have reserved a
        // shorter gap; measuring from A's ETA (pre-#356) would have started from an earlier point.
        assert!(
            cta - a_sta > cruise_gap,
            "pre-#355 sizing would have left only a cruise-sized gap behind the metered crossing"
        );
        assert!(
            cta > metas[1].eta_ms + gap,
            "pre-#356 baselining would have measured the gap from A's ETA, not its metered crossing"
        );
    }

    /// #356, the reported scenario, asserted through the **real decision function**. A release
    /// spaced against A's *ETA* would land 60s behind A's true 120s crossing — half the required
    /// separation. Reverting `rdy_slot` to ETA-based spacing fails here.
    #[test]
    fn a_release_is_spaced_behind_an_arrivals_metered_time_not_its_eta() {
        let metas = leader_arrival_and_departure();
        let metered = fca::meter(&metas, "rate", 30, 0, None);
        let a_sta = metered[1].sched_ms;
        assert_eq!(
            a_sta, 120_000,
            "the ladder must push A back to earn its own spacing behind the leader"
        );

        let cta = rdy_slot(&rate_fca(vec![]), &metas, &metered, 2, false);
        assert!(
            cta - a_sta >= SEP_MS,
            "the release must sit a full separation behind A's metered crossing, got {}s",
            (cta - a_sta) / 1000
        );

        // What the bug did, for contrast: spacing against raw ETAs leaves less than separation.
        let eta_based = fca::earliest_slot(180_000, &[0, 60_000], SEP_MS);
        assert!(eta_based - a_sta < SEP_MS);
    }

    /// The protective margin is part of the decision, not something a caller adds. Dropping
    /// `+ RELEASE_MARGIN_MS` inside `rdy_slot` fails here.
    #[test]
    fn the_release_slot_reserves_the_margin_on_top_of_separation() {
        let metas = leader_arrival_and_departure();
        let metered = fca::meter(&metas, "rate", 30, 0, None);
        let cta = rdy_slot(&rate_fca(vec![]), &metas, &metered, 2, false);
        assert_eq!(
            cta - metered[1].sched_ms,
            SEP_MS + RELEASE_MARGIN_MS,
            "the margin must sit on top of the full separation"
        );
    }

    /// Regression (#356 rework): in **manual** sequence `meter` chains every crossing, so the
    /// releasing ground aircraft pushes the ones behind it later. Spacing the release against those
    /// pushed times is circular — it lands behind an aircraft the controller explicitly ordered it
    /// ahead of. Manual order here is Leader -> Dep -> Trailer, so Dep must keep its slot.
    #[test]
    fn a_manual_release_keeps_the_controllers_order_and_is_not_pushed_by_its_own_wake() {
        let metas = vec![
            cand(0, true, None),        // Leader, airborne
            cand(100_000, false, None), // Dep, ground — the one being released
            cand(110_000, true, None),  // Trailer, airborne, ordered behind Dep
        ];
        let order = vec![0usize, 1, 2];
        let metered = fca::meter(&metas, "rate", 30, 0, Some(&order));
        let (dep_slot, trailer_slot) = (metered[1].sched_ms, metered[2].sched_ms);
        assert_eq!(
            (dep_slot, trailer_slot),
            (120_000, 240_000),
            "sanity: the manual chain pushes Trailer back because Dep sits ahead of it"
        );

        let fca = rate_fca(vec!["LEAD".into(), "DEP".into(), "TRAIL".into()]);
        let cta = rdy_slot(&fca, &metas, &metered, 1, true);

        assert!(
            cta < trailer_slot,
            "the release must stay ahead of the Trailer the controller ordered behind it, got \
             {cta} vs Trailer at {trailer_slot}"
        );
        assert_eq!(
            cta,
            dep_slot + RELEASE_MARGIN_MS,
            "a manual release takes the slot the ladder already assigned it, plus the margin"
        );
    }

    /// `committed_crossings` selects exactly the pinned crossings, excluding the aircraft being
    /// released: unreleased ground traffic has no commitment and floats around the release.
    #[test]
    fn committed_crossings_skips_the_releasing_aircraft_and_unpinned_ground() {
        let metas = vec![
            cand(0, true, None),               // 0 airborne -> committed
            cand(50_000, false, None),         // 1 unreleased ground -> not committed
            cand(90_000, false, Some(90_000)), // 2 issued CFR -> committed
            cand(180_000, false, None),        // 3 the one being released -> excluded
        ];
        let metered = fca::meter(&metas, "rate", 30, 0, None);

        let committed = committed_crossings(&metas, &metered, 3);
        assert_eq!(
            committed,
            vec![metered[0].sched_ms, metered[2].sched_ms],
            "only the airborne crossing and the issued CFR are commitments"
        );
        assert!(
            !committed.contains(&metered[3].sched_ms),
            "the aircraft being released must not be spaced against itself"
        );
    }

    /// `separation_ms` covers both FCA modes, including the `cross_speed` floor.
    #[test]
    fn separation_follows_the_fca_mode() {
        let mut fca = rate_fca(vec![]);
        assert_eq!(separation_ms(&fca, 300.0), SEP_MS);

        fca.mode = "mit".into();
        fca.mit = 20;
        assert_eq!(separation_ms(&fca, 300.0), 240_000); // 20nm / 300kt = 4 min

        fca.mode = "rate".into();
        fca.rate = 0;
        assert_eq!(separation_ms(&fca, 300.0), 0, "no rate set -> no spacing");
    }
}

/// Regression (#355): the FCA metering wiring — not just the primitives. `MeterInput.cross_speed`
/// must be the descent-aware groundspeed `predict::eta_along_route` predicts **at the crossing
/// fix**, because `fca::meter` turns it into a frozen time gap (`MIT ÷ cross_speed`) that the
/// aircraft then flies at its real crossing speed. `predict`'s own tests prove `gs_kt` is
/// position-dependent and `fca`'s prove `meter` divides by whatever it is handed; only this one
/// proves `build_candidates` hands it the right number. Reverting **either** candidate-build site
/// to `trajectory::effective_gs(cruise_tas, headwind)` passes every other test in the repo — so
/// both are covered here: the live-pilot loop via `pilots`, and the prefile loop via `prefiles`.
#[cfg(test)]
mod mit_cross_speed_wiring_tests {
    use std::collections::HashMap;

    use chrono::Utc;

    use super::{ReleaseMap, build_candidates};
    use crate::feed::airports::{Airport, AirportDb};
    use crate::feed::{
        airspace::Boundaries,
        nav::NavData,
        runway_db::RunwayDb,
        trajectory::ProfileTable,
        vatsim::{FlightPlan, Pilot, Prefile, VatsimData},
        winds::Winds,
    };
    use crate::models::FcaBody;

    /// Filed cruise TAS. `capped_cruise_tas(440.0, 35_000.0, &AircraftProfile::default())` returns
    /// exactly this (the default profile sets neither `cruise_tas` nor `cruise_mach`), and
    /// `VerticalProfile::ground_speed_at` caps the cruise band at it, so it *is* the cruise
    /// groundspeed here — not merely a conservative stand-in for one.
    const FILED_TAS: f64 = 440.0;

    /// A 20 MIT FCA whose gate is the vertical line `lon`, spanning `lat_lo..lat_hi`.
    fn mit_fca_at_lon(lon: f64, lat_lo: f64, lat_hi: f64) -> FcaBody {
        FcaBody {
            id: "t".into(),
            name: "t".into(),
            color: "#fff".into(),
            artcc: "ZDC".into(),
            points: sqlx::types::Json(vec![[lat_lo, lon], [lat_hi, lon]]),
            dests: vec![],
            origins: vec![],
            fixes: vec![],
            scope: vec![],
            min_fl: None,
            max_fl: None,
            dir: "any".into(),
            mode: "mit".into(),
            rate: 0,
            mit: 20,
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

    fn jfk_dca_plan() -> FlightPlan {
        FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            route: "".into(),
            altitude: "35000".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        }
    }

    /// One airborne jet established at FL350 direct KJFK→KDCA (~164 nm out), **and** a prefile on
    /// the same route. `build_candidates` has two separate candidate-build loops with their own
    /// `cross_speed` assignment; a fixture with only `pilots` leaves the prefile one unexercised,
    /// so a revert there would pass unnoticed.
    fn arrival_at_cruise() -> VatsimData {
        VatsimData {
            pilots: vec![Pilot {
                callsign: "AAL1".into(),
                latitude: 40.5,
                longitude: -74.2,
                altitude: 35_000,
                groundspeed: 440,
                heading: 220,
                flight_plan: Some(jfk_dca_plan()),
                ..Default::default()
            }],
            prefiles: vec![Prefile {
                callsign: "AAL2".into(),
                flight_plan: Some(jfk_dca_plan()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn airports() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ])
    }

    fn cross_speed_for(fca: &FcaBody) -> (f64, f64) {
        let nav = NavData::load();
        let (flights, metas) = build_candidates(
            fca,
            &arrival_at_cruise(),
            &airports(),
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
        assert_eq!(
            flights.len(),
            2,
            "both the live pilot and the prefile must cross this FCA, got {:?}",
            flights.iter().map(|f| &f.callsign).collect::<Vec<_>>()
        );
        // (live-pilot loop, prefile loop) — the two independent `cross_speed` sites.
        let idx = |cs: &str| flights.iter().position(|f| f.callsign == cs).unwrap();
        (
            metas[idx("AAL1")].cross_speed,
            metas[idx("AAL2")].cross_speed,
        )
    }

    #[test]
    fn cross_speed_at_a_low_arrival_fix_is_the_descent_speed_not_cruise() {
        // Gate ~17 nm from KDCA — deep in the descent.
        let (near, near_prefile) = cross_speed_for(&mit_fca_at_lon(-76.75, 38.5, 39.5));
        for (site, near) in [("live pilot", near), ("prefile", near_prefile)] {
            assert!(
                near < FILED_TAS * 0.8,
                "{site}: crossing speed at a low arrival fix was {near:.0} kt — not well below the \
             {FILED_TAS:.0} kt cruise groundspeed, so the gap is being sized with a speed the \
             aircraft no longer has by the fix, under-provisioning every crossing (#355)"
            );
            // …and it is a plausible arrival speed, not merely "some smaller number".
            assert!(
                (120.0..=350.0).contains(&near),
                "{site}: crossing speed {near:.0} kt is outside a plausible arrival band"
            );
        }
    }

    #[test]
    fn cross_speed_at_an_enroute_fix_is_still_cruise() {
        // Gate ~53 nm along the KJFK→KDCA route. The live pilot is already past it at FL350, so
        // it must report cruise. The prefile has not left KJFK, so the same gate sits in its
        // *climb* — a legitimately lower speed, and the reason the two sites are asserted apart
        // rather than lumped together.
        let (far, far_prefile) = cross_speed_for(&mit_fca_at_lon(-74.6, 39.5, 41.0));

        assert!(
            far > FILED_TAS * 0.9,
            "live pilot: an enroute crossing must still be sized at cruise, got {far:.0} kt"
        );
        assert!(
            (150.0..FILED_TAS).contains(&far_prefile),
            "prefile: a gate this close to its departure is a climb crossing, so it should sit \
             below the {FILED_TAS:.0} kt cruise but in a plausible climb band, got {far_prefile:.0} kt"
        );

        // #355 follow-up, both sites: never *above* cruise. The profile's cruise-band TAS ramp
        // used to read ~508 kt here, which under-provisions a 20 MIT fix to 17.3 real nm.
        for (site, gs) in [("live pilot", far), ("prefile", far_prefile)] {
            assert!(
                gs <= FILED_TAS + 1.0,
                "{site}: must not read above the {FILED_TAS:.0} kt cruise groundspeed, got {gs:.0} kt"
            );
        }
    }

    /// **A departure/climb gate is metered on its climb speed, and that is intended.**
    ///
    /// Raised at review as an unremarked consequence of #355: a 20 MIT gate shortly after departure
    /// used to be sized at cruise (163.6 s) and is now sized at the climb speed the aircraft
    /// actually crosses at (259.9 s here) — a large throughput reduction at departure gates.
    ///
    /// Confirmed as intended. MIT is a distance *at the fix*: a departure climbing through it at
    /// ~277 kt genuinely needs the longer gap to end up 20 nm in trail. The old 163.6 s under-spaced
    /// departure gates for exactly the same reason it under-spaced arrivals — this is the same bug,
    /// not a new one. Pinned as a test so the decision travels with the code, and so anyone who
    /// later reads the throughput drop as a regression finds the reasoning attached to it.
    #[test]
    fn a_departure_gate_is_metered_on_its_climb_speed_by_design() {
        // A gate ~20 nm west of KJFK. Only the prefile crosses it — the live pilot is already past.
        let nav = NavData::load();
        let (flights, metas) = build_candidates(
            &mit_fca_at_lon(-74.05, 38.0, 41.5),
            &arrival_at_cruise(),
            &airports(),
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
        let i = flights
            .iter()
            .position(|f| f.callsign == "AAL2")
            .expect("the departing prefile must cross a gate just off its departure field");
        let climb = metas[i].cross_speed;

        assert!(
            climb < FILED_TAS,
            "a departure gate must be sized on the climb speed it is crossed at, not the \
             {FILED_TAS:.0} kt filed cruise, got {climb:.0} kt"
        );

        // The throughput change that was flagged, asserted rather than described. Both figures use
        // `mit / speed * 3600` — the same formula `fca::meter`'s `sep_ms` applies.
        let gap = |gs: f64| 20.0 / gs * 3600.0;
        let (cruise_sized, climb_sized) = (gap(FILED_TAS), gap(climb));
        assert!(
            climb_sized > cruise_sized * 1.4,
            "expected a materially longer gap at a departure gate: cruise-sized {cruise_sized:.1}s \
             vs climb-sized {climb_sized:.1}s"
        );

        // And it is the *correct* longer gap: a departure released into it ends up the configured
        // 20 MIT in trail at the speed it is actually doing. That is the whole point of #355.
        let realized_nm = climb_sized / 3600.0 * climb;
        assert!(
            (realized_nm - 20.0).abs() < 0.5,
            "a departure released into this gap must end up the configured 20 MIT in trail, got \
             {realized_nm:.1} nm"
        );
    }

    /// The debug view has to be able to explain the gap. Since #355 the gap is a function of the
    /// crossing speed, not `cruise_tas`, so `FcaFlightDebug` reports both — and the reported
    /// `cross_speed` must be the *same* number the metering used, floor included.
    #[test]
    fn the_debug_view_reports_the_crossing_speed_that_sized_the_gap() {
        let fca = mit_fca_at_lon(-76.75, 38.5, 39.5); // low arrival gate
        let nav = NavData::load();
        let (flights, metas) = build_candidates(
            &fca,
            &arrival_at_cruise(),
            &airports(),
            &nav,
            &Boundaries::default(),
            &Winds::default(),
            &ProfileTable::default(),
            &ReleaseMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            Utc::now(),
            true, // debug on
        );

        for (f, m) in flights.iter().zip(&metas) {
            let dbg = f
                .debug
                .as_ref()
                .unwrap_or_else(|| panic!("{}: debug requested but absent", f.callsign));
            assert_eq!(
                dbg.cross_speed,
                m.cross_speed.round() as i64,
                "{}: the debug view must report the crossing speed metering actually used",
                f.callsign
            );
            assert!(
                dbg.cross_speed < dbg.cruise_tas,
                "{}: at a low arrival fix the crossing speed ({} kt) should be below cruise ({} kt) \
                 — reporting only cruise is what made the gap unexplainable",
                f.callsign,
                dbg.cross_speed,
                dbg.cruise_tas
            );
        }
    }
}

#[cfg(test)]
mod manual_exclusion_tests {
    use super::{ExclusionSet, all_excluded_callsigns, is_manually_excluded};
    use std::collections::HashSet;

    fn exclusions() -> ExclusionSet {
        ExclusionSet::from([
            ("ZDC".to_string(), HashSet::from(["BOGUS1".to_string()])),
            ("ZNY".to_string(), HashSet::from(["BOGUS2".to_string()])),
        ])
    }

    /// #342: the FCA surfaces (crossings, metering, badge counts) scope the check to the FCA's own
    /// ARTCC, so one facility's removal doesn't silently edit another facility's board.
    #[test]
    fn an_artcc_scoped_check_only_sees_that_facilitys_removals() {
        let ex = exclusions();
        assert!(is_manually_excluded(&ex, Some("ZDC"), "BOGUS1"));
        assert!(
            !is_manually_excluded(&ex, Some("ZDC"), "BOGUS2"),
            "ZNY's removal must not apply to a ZDC FCA"
        );
        assert!(!is_manually_excluded(&ex, Some("ZDC"), "UAL123"));
        assert!(
            !is_manually_excluded(&ex, Some("ZAB"), "BOGUS1"),
            "a facility with no removals excludes nothing"
        );
    }

    /// The global traffic endpoints carry no facility context, so they ask "excluded anywhere?" —
    /// that is what actually clears a bogus flight off the map for every viewer.
    #[test]
    fn an_unscoped_check_sees_every_facilitys_removals() {
        let ex = exclusions();
        assert!(is_manually_excluded(&ex, None, "BOGUS1"));
        assert!(is_manually_excluded(&ex, None, "BOGUS2"));
        assert!(!is_manually_excluded(&ex, None, "UAL123"));
    }

    #[test]
    fn an_empty_set_excludes_nothing() {
        let ex = ExclusionSet::new();
        assert!(!is_manually_excluded(&ex, Some("ZDC"), "BOGUS1"));
        assert!(!is_manually_excluded(&ex, None, "BOGUS1"));
    }

    /// `feed::flow::compute` takes the flat form, since airport flow / AADC demand has no single
    /// facility context.
    #[test]
    fn flattening_collects_every_facilitys_callsigns() {
        let flat = all_excluded_callsigns(&exclusions());
        assert_eq!(
            flat,
            HashSet::from(["BOGUS1".to_string(), "BOGUS2".to_string()])
        );
        assert!(all_excluded_callsigns(&ExclusionSet::new()).is_empty());
    }
}

/// #360: the FCA inclusion decision lives in exactly one place. The metering board, the badge
/// counts and the per-flight advisory used to restate it separately and could drift apart.
#[cfg(test)]
mod fca_inclusion_tests {
    use std::collections::{HashMap, HashSet};

    use chrono::Utc;

    use super::{ExclusionSet, ReleaseMap, build_candidates, fca_crossing_for};
    use crate::feed::airports::{Airport, AirportDb};
    use crate::feed::{
        airspace::Boundaries,
        fca,
        nav::NavData,
        runway_db::RunwayDb,
        trajectory::ProfileTable,
        vatsim::{FlightPlan, Pilot, VatsimData},
        winds::Winds,
    };
    use crate::models::FcaBody;

    fn fca_at(lon_lo: f64, lon_hi: f64) -> FcaBody {
        FcaBody {
            id: "t".into(),
            name: "t".into(),
            color: "#fff".into(),
            artcc: "ZDC".into(),
            points: sqlx::types::Json(vec![[39.5, lon_lo], [39.5, lon_hi]]),
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

    fn plan() -> FlightPlan {
        FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            route: "".into(),
            altitude: "35000".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        }
    }

    fn airports() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ])
    }

    fn data() -> VatsimData {
        VatsimData {
            pilots: vec![Pilot {
                callsign: "AAL1".into(),
                latitude: 40.5,
                longitude: -74.2,
                altitude: 35_000,
                groundspeed: 440,
                heading: 220,
                flight_plan: Some(plan()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn path(nav: &NavData, ap: &AirportDb) -> Vec<[f64; 2]> {
        let fp = plan();
        fca::route_path(
            nav,
            ap,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            40.5,
            -74.2,
            220,
            440,
        )
        .expect("the JFK->DCA corridor must resolve")
    }

    /// The `passes_scope` arm, with **real** ARTCC boundaries (#360 rework).
    ///
    /// `passes_scope` short-circuits to `true` when `fca.scope` is empty, and every other fixture
    /// here leaves it empty — so the arm never executed and deleting it from `fca_crossing_for`
    /// kept the whole suite green. For a refactor whose product is "one function owns every reason
    /// a flight is left out", that one reason has to be under test too.
    ///
    /// The fixture gate crosses the JFK→DCA corridor at lat 39.5, which the bundled boundaries put
    /// inside **ZDC**. Scoping the FCA to ZDC must include the flight; scoping it to an ARTCC the
    /// crossing is nowhere near must exclude it.
    #[test]
    fn a_scoped_fca_only_matches_a_crossing_inside_its_airspace() {
        let nav = NavData::load();
        let ap = airports();
        let path = path(&nav, &ap);
        let airspace = Boundaries::load();
        assert!(
            !airspace.is_empty(),
            "sanity: real boundaries must load, or passes_scope short-circuits and proves nothing"
        );

        let scoped_to = |zone: &str| {
            let mut fca = fca_at(-76.5, -73.0);
            fca.scope = vec![zone.to_string()];
            fca_crossing_for(
                &fca,
                &airspace,
                &ExclusionSet::new(),
                "AAL1",
                &plan(),
                Some(35_000),
                &path,
            )
        };

        // Sanity: the crossing really is in ZDC, so an unscoped FCA matches it.
        assert!(
            fca_crossing_for(
                &fca_at(-76.5, -73.0),
                &airspace,
                &ExclusionSet::new(),
                "AAL1",
                &plan(),
                Some(35_000),
                &path,
            )
            .is_some(),
            "sanity: the corridor crossing must be included when no scope is set"
        );

        assert!(
            scoped_to("ZDC").is_some(),
            "an FCA scoped to the ARTCC its crossing lies in must include the flight"
        );
        assert!(
            scoped_to("ZLA").is_none(),
            "an FCA scoped to an ARTCC the crossing is outside must exclude the flight — if this \
             passes, `passes_scope` is not being consulted"
        );
    }

    /// Every reason a flight is left out is one function's answer, so adding a condition can't
    /// reach one surface and miss another.
    #[test]
    fn the_decision_covers_exclusion_filters_crossing_and_scope() {
        let nav = NavData::load();
        let ap = airports();
        let path = path(&nav, &ap);
        let empty = ExclusionSet::new();
        let fp = plan();
        let cross = |fca: &FcaBody, ex: &ExclusionSet, alt: Option<i64>| {
            fca_crossing_for(fca, &Boundaries::default(), ex, "AAL1", &fp, alt, &path)
        };

        // Baseline: an ordinary crossing is included, and the geometry comes back with it.
        let hit = cross(&fca_at(-76.5, -73.0), &empty, Some(35_000))
            .expect("the corridor FCA must include this flight");
        assert!(
            hit.along_nm > 0.0,
            "the decision must hand back the crossing, not just a bool"
        );

        // Manually excluded for this FCA's facility (#342).
        let excluded =
            ExclusionSet::from([("ZDC".to_string(), HashSet::from(["AAL1".to_string()]))]);
        assert!(
            cross(&fca_at(-76.5, -73.0), &excluded, Some(35_000)).is_none(),
            "a manually excluded callsign must be left out"
        );
        // …but another facility's removal must not affect a ZDC FCA.
        let other = ExclusionSet::from([("ZNY".to_string(), HashSet::from(["AAL1".to_string()]))]);
        assert!(
            cross(&fca_at(-76.5, -73.0), &other, Some(35_000)).is_some(),
            "another facility's removal must not touch this FCA"
        );

        // Membership filters: a destination this flight doesn't match.
        let mut wrong_dest = fca_at(-76.5, -73.0);
        wrong_dest.dests = vec!["KBOS".into()];
        assert!(
            cross(&wrong_dest, &empty, Some(35_000)).is_none(),
            "a flight failing the membership filters must be left out"
        );

        // Altitude band it sits outside of, on both filed and current.
        let mut wrong_band = fca_at(-76.5, -73.0);
        wrong_band.min_fl = Some(400);
        assert!(
            cross(&wrong_band, &empty, Some(35_000)).is_none(),
            "a flight outside the altitude band must be left out"
        );

        // Geometry: a line the route never reaches.
        assert!(
            cross(&fca_at(-60.0, -59.0), &empty, Some(35_000)).is_none(),
            "a line the route doesn't cross must be left out"
        );
    }

    /// The metering board and the shared decision cannot disagree: `build_candidates` includes
    /// exactly the flights `fca_crossing_for` accepts, and at the same crossing distance.
    #[test]
    fn the_metering_board_matches_the_shared_decision() {
        let nav = NavData::load();
        let ap = airports();
        let path = path(&nav, &ap);
        let fp = plan();
        let empty = ExclusionSet::new();

        for (label, fca) in [
            ("crossed", fca_at(-76.5, -73.0)),
            ("not crossed", fca_at(-60.0, -59.0)),
        ] {
            let decision = fca_crossing_for(
                &fca,
                &Boundaries::default(),
                &empty,
                "AAL1",
                &fp,
                Some(35_000),
                &path,
            );
            let (flights, _) = build_candidates(
                &fca,
                &data(),
                &ap,
                &nav,
                &Boundaries::default(),
                &Winds::default(),
                &ProfileTable::default(),
                &ReleaseMap::new(),
                &HashMap::new(),
                &RunwayDb::default(),
                &HashMap::new(),
                &empty,
                Utc::now(),
                false,
            );
            assert_eq!(
                decision.is_some(),
                !flights.is_empty(),
                "{label}: the board and the shared decision must agree on inclusion"
            );
            // Guard against agreeing vacuously: the "crossed" case must actually be included.
            assert_eq!(
                decision.is_some(),
                label == "crossed",
                "{label}: fixture no longer exercises what it claims to"
            );
            if let Some(c) = decision {
                assert!(
                    (flights[0].distance_nm as f64 - c.along_nm).abs() < 1.0,
                    "{label}: the board must meter the crossing the decision returned"
                );
            }
        }
    }
}
