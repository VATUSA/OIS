//! Read API over the persisted VATSIM stats (`/api/v1/stats/*`). Ported from the standalone stats
//! system's API, gated on `stats.read`. Historical lookups only — "now" is served by the live feed.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::{permissions::StatsRead, require_permission::RequirePermission},
    errors::ApiError,
    feed::stats::reconstruct::reconstruct_at,
    handlers::{atc, feed as feed_handlers, flow as flow_handlers, runway as runway_handlers},
    models::{
        AtcBoard, CaptureSummaryBody, DelaySummary, DeparturesResponse, NetworkPointBody,
        ReplayBody, ReplayChunkBody, ReplayFlightBody, ReplayPlan, StatsAirportBody,
        StatsFlightDetail, StatsFlightSummary, StatsTrackBody, TrafficAircraft,
    },
    repos::stats as stats_repo,
    state::AppState,
};

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

/// (callsign, departure, arrival, aircraft, route) for a replay flight.
type FlightMeta = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn norm_icao(raw: &str) -> String {
    raw.trim().to_ascii_uppercase()
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/network/history",
    tag = "stats",
    params(
        ("from" = Option<String>, Query, description = "RFC3339 start (default 7d ago)"),
        ("to" = Option<String>, Query, description = "RFC3339 end (default now)")
    ),
    responses((status = 200, body = Vec<NetworkPointBody>), (status = 401))
)]
pub async fn network_history(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<NetworkPointBody>>, ApiError> {
    let to = q.to.unwrap_or_else(Utc::now);
    let from = q.from.unwrap_or(to - Duration::days(7));
    Ok(Json(
        stats_repo::network_history(pool(&state)?, from, to).await?,
    ))
}

#[derive(Deserialize)]
pub struct LimitQuery {
    limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/airports/top",
    tag = "stats",
    params(("limit" = Option<i64>, Query, description = "Max airports (default 20)")),
    responses((status = 200, body = Vec<crate::models::KeyCountBody>), (status = 401))
)]
pub async fn airports_top(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<crate::models::KeyCountBody>>, ApiError> {
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    Ok(Json(stats_repo::airports_top(pool(&state)?, limit).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/airports/{icao}",
    tag = "stats",
    params(("icao" = String, Path, description = "Airport ICAO")),
    responses((status = 200, body = StatsAirportBody), (status = 401))
)]
pub async fn airport_stats(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(icao): Path<String>,
) -> Result<Json<StatsAirportBody>, ApiError> {
    let p = pool(&state)?;
    let icao = norm_icao(&icao);
    let (departures, arrivals) = stats_repo::airport_counts(p, &icao).await?;
    Ok(Json(StatsAirportBody {
        top_aircraft: stats_repo::airport_top_aircraft(p, &icao).await?,
        top_destinations: stats_repo::airport_top_endpoints(p, &icao, false).await?,
        top_origins: stats_repo::airport_top_endpoints(p, &icao, true).await?,
        departures,
        arrivals,
        icao,
    }))
}

#[derive(Deserialize)]
pub struct DelayQuery {
    /// `departure` (taxi-out) or `arrival` (transit); default departure.
    kind: Option<String>,
    /// Filter to one airport (also enables the per-runway / per-procedure breakdowns).
    airport: Option<String>,
    runway: Option<String>,
    procedure: Option<String>,
    /// Rolling window, hours back (default 24, max 720).
    hours: Option<i64>,
}

fn norm_opt(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_ascii_uppercase())
        .filter(|v| !v.is_empty())
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/delays",
    tag = "stats",
    params(
        ("kind" = Option<String>, Query, description = "departure | arrival (default departure)"),
        ("airport" = Option<String>, Query, description = "Filter to one airport ICAO"),
        ("runway" = Option<String>, Query, description = "Filter to one runway"),
        ("procedure" = Option<String>, Query, description = "Filter to one SID/STAR"),
        ("hours" = Option<i64>, Query, description = "Window hours back (default 24, max 720)")
    ),
    responses((status = 200, body = DelaySummary), (status = 401))
)]
pub async fn delay_summary(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<DelayQuery>,
) -> Result<Json<DelaySummary>, ApiError> {
    let kind = if q.kind.as_deref() == Some("arrival") {
        "arrival"
    } else {
        "departure"
    };
    let hours = q.hours.unwrap_or(24).clamp(1, 720);
    let since = Utc::now() - Duration::hours(hours);
    let airport = norm_opt(q.airport);
    let runway = norm_opt(q.runway);
    let procedure = norm_opt(q.procedure);
    Ok(Json(
        stats_repo::delay_summary(
            pool(&state)?,
            kind,
            airport.as_deref(),
            runway.as_deref(),
            procedure.as_deref(),
            since,
            hours,
        )
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct MovementsQuery {
    /// `arr` (arrivals) or `dep` (departures, default).
    dir: Option<String>,
    limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/airports/{icao}/movements",
    tag = "stats",
    params(
        ("icao" = String, Path, description = "Airport ICAO"),
        ("dir" = Option<String>, Query, description = "arr | dep (default dep)"),
        ("limit" = Option<i64>, Query, description = "Max rows (default 50)")
    ),
    responses((status = 200, body = Vec<StatsFlightSummary>), (status = 401))
)]
pub async fn airport_movements(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(icao): Path<String>,
    Query(q): Query<MovementsQuery>,
) -> Result<Json<Vec<StatsFlightSummary>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let arrivals = q.dir.as_deref() == Some("arr");
    Ok(Json(
        stats_repo::airport_movements(pool(&state)?, &norm_icao(&icao), arrivals, limit).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/members/{cid}/flights",
    tag = "stats",
    params(
        ("cid" = i32, Path, description = "VATSIM CID"),
        ("limit" = Option<i64>, Query, description = "Max rows (default 50)")
    ),
    responses((status = 200, body = Vec<StatsFlightSummary>), (status = 401))
)]
pub async fn member_flights(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(cid): Path<i32>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Vec<StatsFlightSummary>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(
        stats_repo::member_flights(pool(&state)?, cid, limit).await?,
    ))
}

/// Session ids are large i64 hashes; they travel as strings to survive JS number precision.
fn parse_session_id(raw: &str) -> Result<i64, ApiError> {
    raw.trim().parse::<i64>().map_err(|_| ApiError::BadRequest)
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/flights/{id}",
    tag = "stats",
    params(("id" = String, Path, description = "Flight session id")),
    responses((status = 200, body = StatsFlightDetail), (status = 400), (status = 401), (status = 404))
)]
pub async fn flight_detail(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(id): Path<String>,
) -> Result<Json<StatsFlightDetail>, ApiError> {
    let p = pool(&state)?;
    let sid = parse_session_id(&id)?;
    let mut detail = stats_repo::flight_detail(p, sid)
        .await?
        .ok_or(ApiError::NotFound)?;
    detail.revisions = stats_repo::flight_plan_history(p, sid).await?;
    Ok(Json(detail))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/flights/{id}/track",
    tag = "stats",
    params(("id" = String, Path, description = "Flight session id")),
    responses((status = 200, body = StatsTrackBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn flight_track(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(id): Path<String>,
) -> Result<Json<StatsTrackBody>, ApiError> {
    let p = pool(&state)?;
    let id = parse_session_id(&id)?;
    let raw = stats_repo::flight_track_raw(p, id).await?;
    if !raw.is_empty() {
        let points: Vec<Value> = raw
            .into_iter()
            .map(|(ts, lat, lon, alt, gs, hdg)| {
                json!({ "ts": ts, "lat": lat, "lon": lon, "alt": alt, "gs": gs, "hdg": hdg })
            })
            .collect();
        return Ok(Json(StatsTrackBody {
            resolution: "full".into(),
            points: Value::Array(points),
        }));
    }
    // Fall back to the stored simplified path; 404 only when the flight itself is unknown.
    match stats_repo::flight_path_simplified(p, id).await? {
        Some(path) => Ok(Json(StatsTrackBody {
            resolution: "simplified".into(),
            points: path,
        })),
        None => {
            if stats_repo::flight_detail(p, id).await?.is_none() {
                return Err(ApiError::NotFound);
            }
            Ok(Json(StatsTrackBody {
                resolution: "none".into(),
                points: Value::Array(Vec::new()),
            }))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/captures",
    tag = "stats",
    responses((status = 200, body = Vec<CaptureSummaryBody>), (status = 401))
)]
pub async fn list_captures(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
) -> Result<Json<Vec<CaptureSummaryBody>>, ApiError> {
    Ok(Json(
        stats_repo::list_replayable_captures(pool(&state)?).await?,
    ))
}

#[derive(Deserialize)]
pub struct ReplayQuery {
    /// Sample spacing in seconds (default 30, clamped 15–300).
    step: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/captures/{id}/replay",
    tag = "stats",
    params(
        ("id" = String, Path, description = "Capture id"),
        ("step" = Option<i64>, Query, description = "Sample spacing seconds (default 30)")
    ),
    responses((status = 200, body = ReplayBody), (status = 401), (status = 404))
)]
pub async fn capture_replay(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(id): Path<String>,
    Query(q): Query<ReplayQuery>,
) -> Result<Json<ReplayBody>, ApiError> {
    let p = pool(&state)?;
    let cap = stats_repo::capture_get(p, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let from = cap.start_time;
    let to = cap.end_time.unwrap_or_else(Utc::now);
    let step = q.step.unwrap_or(30).clamp(15, 300);
    Ok(Json(build_replay(p, cap.id, from, to, step).await?))
}

#[derive(Deserialize)]
pub struct WindowReplayQuery {
    /// Window start (Unix epoch seconds).
    from: i64,
    /// Window end (Unix epoch seconds).
    to: i64,
    /// Sample spacing in seconds (default 30, clamped 15–300).
    step: Option<i64>,
}

/// Replay an arbitrary `[from, to]` window on the map (not tied to a saved capture) — same
/// per-flight thinned tracks the capture replay returns, so the deck.gl player is identical.
#[utoipa::path(
    get,
    path = "/api/v1/stats/replay",
    tag = "stats",
    params(
        ("from" = i64, Query, description = "Window start (Unix epoch seconds)"),
        ("to" = i64, Query, description = "Window end (Unix epoch seconds)"),
        ("step" = Option<i64>, Query, description = "Sample spacing seconds (default 30)")
    ),
    responses((status = 200, body = ReplayBody), (status = 400), (status = 401))
)]
pub async fn window_replay(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<WindowReplayQuery>,
) -> Result<Json<ReplayBody>, ApiError> {
    let p = pool(&state)?;
    let from = DateTime::from_timestamp(q.from, 0).ok_or(ApiError::BadRequest)?;
    let to = DateTime::from_timestamp(q.to, 0).ok_or(ApiError::BadRequest)?;
    if to <= from {
        return Err(ApiError::BadRequest);
    }
    let step = q.step.unwrap_or(30).clamp(15, 300);
    Ok(Json(build_replay(p, String::new(), from, to, step).await?))
}

/// Group ordered `(session, t)` samples into per-flight tracks and attach each flight's callsign +
/// the flight-plan revisions in force over `[plan_from, plan_to]` (so a mid-route amendment shows the
/// plan that was actually in effect at each instant). Shared by the whole-window and chunked paths.
async fn assemble_flights(
    p: &sqlx::PgPool,
    samples: Vec<stats_repo::ReplaySample>,
    plan_from: DateTime<Utc>,
    plan_to: DateTime<Utc>,
) -> Result<Vec<ReplayFlightBody>, ApiError> {
    // Group consecutive samples (already ordered by session_id, then time) into per-flight tracks.
    let mut flights: Vec<ReplayFlightBody> = Vec::new();
    let mut ids: Vec<i64> = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        let sid = samples[i].session_id;
        let mut track: Vec<[f64; 6]> = Vec::new();
        while i < samples.len() && samples[i].session_id == sid {
            let s = &samples[i];
            track.push([
                s.t,
                s.lat as f64,
                s.lon as f64,
                s.alt as f64,
                s.heading as f64,
                s.gs as f64,
            ]);
            i += 1;
        }
        ids.push(sid);
        flights.push(ReplayFlightBody {
            session_id: sid,
            callsign: String::new(),
            plans: Vec::new(),
            samples: track,
        });
    }

    let meta: HashMap<i64, FlightMeta> = stats_repo::flights_meta(p, &ids)
        .await?
        .into_iter()
        .map(|(sid, cs, dep, arr, ac, route)| (sid, (cs, dep, arr, ac, route)))
        .collect();
    let mut plans_by_sid: HashMap<i64, Vec<ReplayPlan>> = HashMap::new();
    for (sid, eff, dep, arr, ac, route) in
        stats_repo::flight_plan_revisions(p, &ids, plan_from, plan_to).await?
    {
        plans_by_sid.entry(sid).or_default().push(ReplayPlan {
            t: (eff - plan_from).num_seconds().max(0) as f64,
            departure: dep,
            arrival: arr,
            aircraft: ac,
            route,
        });
    }
    for f in flights.iter_mut() {
        if let Some((cs, dep, arr, ac, route)) = meta.get(&f.session_id) {
            f.callsign = cs.clone();
            // Recorded revisions when we have them; otherwise a single plan (pre-0044 captures).
            f.plans = plans_by_sid.remove(&f.session_id).unwrap_or_else(|| {
                vec![ReplayPlan {
                    t: 0.0,
                    departure: dep.clone(),
                    arrival: arr.clone(),
                    aircraft: ac.clone(),
                    route: route.clone(),
                }]
            });
        } else if let Some(plans) = plans_by_sid.remove(&f.session_id) {
            f.plans = plans;
        }
    }
    Ok(flights)
}

/// Build the whole-window replay payload in one shot (legacy endpoints). For long windows prefer the
/// progressive `/stats/replay/positions` chunk endpoint, which only scans one chunk at a time.
async fn build_replay(
    p: &sqlx::PgPool,
    capture_id: String,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    step: i64,
) -> Result<ReplayBody, ApiError> {
    let samples = stats_repo::replay_positions(p, from, from, to, step).await?;
    let flights = assemble_flights(p, samples, from, to).await?;
    Ok(ReplayBody {
        capture_id,
        window_start: from,
        window_end: to,
        step_s: step,
        flights,
    })
}

/// Sample spacing chosen from the window length so a replay's total sample count stays bounded no
/// matter how long the span is. The frontend mirrors this to size its chunks; the server clamps.
pub fn adaptive_step(window_secs: i64) -> i64 {
    match window_secs {
        s if s <= 2 * 3600 => 15,
        s if s <= 6 * 3600 => 30,
        s if s <= 24 * 3600 => 60,
        s if s <= 72 * 3600 => 120,
        _ => 300,
    }
}

#[derive(Deserialize)]
pub struct ChunkQuery {
    /// Window start (Unix seconds) — the origin `t` is measured from, and the lower bound for plans.
    from: i64,
    /// Window end (Unix seconds) — the upper bound for the plan-revision timeline.
    to: i64,
    /// This chunk's start (Unix seconds).
    cfrom: i64,
    /// This chunk's end (Unix seconds), exclusive.
    cto: i64,
    /// Sample spacing seconds; default is derived from the window length, clamped 15–300.
    step: Option<i64>,
}

/// One chunk `[cfrom, cto)` of a progressive replay: per-flight samples (with `t` relative to the
/// window start `from`, so chunks stitch together) plus the callsign and full-window plan timeline
/// for the flights that appear in this chunk. The frontend fetches chunks as the clock advances.
#[utoipa::path(
    get,
    path = "/api/v1/stats/replay/positions",
    tag = "stats",
    params(
        ("from" = i64, Query, description = "Window start (Unix seconds)"),
        ("to" = i64, Query, description = "Window end (Unix seconds)"),
        ("cfrom" = i64, Query, description = "Chunk start (Unix seconds)"),
        ("cto" = i64, Query, description = "Chunk end (Unix seconds, exclusive)"),
        ("step" = Option<i64>, Query, description = "Sample spacing seconds (default: adaptive)")
    ),
    responses((status = 200, body = ReplayChunkBody), (status = 400), (status = 401))
)]
pub async fn replay_chunk(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<ChunkQuery>,
) -> Result<Json<ReplayChunkBody>, ApiError> {
    let p = pool(&state)?;
    let from = DateTime::from_timestamp(q.from, 0).ok_or(ApiError::BadRequest)?;
    let to = DateTime::from_timestamp(q.to, 0).ok_or(ApiError::BadRequest)?;
    let cfrom = DateTime::from_timestamp(q.cfrom, 0).ok_or(ApiError::BadRequest)?;
    let cto = DateTime::from_timestamp(q.cto, 0).ok_or(ApiError::BadRequest)?;
    if to <= from || cto <= cfrom {
        return Err(ApiError::BadRequest);
    }
    let step = q
        .step
        .unwrap_or_else(|| adaptive_step((to - from).num_seconds()))
        .clamp(15, 300);
    let samples = stats_repo::replay_positions(p, from, cfrom, cto, step).await?;
    let flights = assemble_flights(p, samples, from, to).await?;
    Ok(Json(ReplayChunkBody {
        step_s: step,
        flights,
    }))
}

// --- historical ("time-machine") dashboard: live feed compute functions replayed at instant T ---
//
// Each endpoint reconstructs the network snapshot at `?at=<unix seconds>` from the stats tables and
// runs the SAME pure compute function the live feed endpoint uses, returning the same body type.
// The frontend dashboard's data-source widgets branch to these when in historical mode.

#[derive(Deserialize)]
pub struct AtQuery {
    /// Instant to reconstruct, as Unix epoch seconds.
    at: i64,
}

/// Resolve the `?at=` epoch to a UTC instant (400 if out of range).
fn parse_at(q: &AtQuery) -> Result<DateTime<Utc>, ApiError> {
    DateTime::from_timestamp(q.at, 0).ok_or(ApiError::BadRequest)
}

/// The winds snapshot to meter a past flow/runway against — the nearest one at or before `at`,
/// falling back to still air when nothing was captured that far back.
async fn winds_for(
    pool: &sqlx::PgPool,
    at: DateTime<Utc>,
) -> Result<crate::feed::winds::Winds, ApiError> {
    Ok(stats_repo::winds_at(pool, at).await?.unwrap_or_default())
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/flow/{icao}",
    tag = "stats",
    params(
        ("icao" = String, Path, description = "Arrival airport ICAO"),
        ("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")
    ),
    responses((status = 200, body = crate::feed::flow::Flow), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_flow(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(icao): Path<String>,
    Query(q): Query<AtQuery>,
) -> Result<Json<crate::feed::flow::Flow>, ApiError> {
    let p = pool(&state)?;
    let at = parse_at(&q)?;
    let icao = norm_icao(&icao);
    let data = reconstruct_at(p, at).await?;
    let winds = winds_for(p, at).await?;
    let snap = Arc::new(crate::feed::Snapshot::of(data));
    Ok(Json(
        feed_handlers::flow_from_data(&state, p, &icao, snap, Arc::new(winds), at).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/departures/{dep}",
    tag = "stats",
    params(
        ("dep" = String, Path, description = "Departure field: airport, TRACON, or ARTCC"),
        ("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")
    ),
    responses((status = 200, body = DeparturesResponse), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_departures(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(dep): Path<String>,
    Query(q): Query<AtQuery>,
) -> Result<Json<DeparturesResponse>, ApiError> {
    let p = pool(&state)?;
    let at = parse_at(&q)?;
    let dep = norm_icao(&dep);
    let data = reconstruct_at(p, at).await?;
    let winds = winds_for(p, at).await?;
    let snap = Arc::new(crate::feed::Snapshot::of(data));
    Ok(Json(
        feed_handlers::departures_response(&state, p, &dep, snap, Arc::new(winds), at).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/atc",
    tag = "stats",
    params(("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")),
    responses((status = 200, body = AtcBoard), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_atc(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<AtQuery>,
) -> Result<Json<AtcBoard>, ApiError> {
    let p = pool(&state)?;
    let at = parse_at(&q)?;
    let data = reconstruct_at(p, at).await?;
    let (airports, iata) = {
        let guard = state.feed.read().await;
        (guard.airports.clone(), guard.iata.clone())
    };
    let tracons = state.tracons.load();
    Ok(Json(atc::board_from(&data, &airports, &iata, &tracons)))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/traffic",
    tag = "stats",
    params(("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")),
    responses((status = 200, body = Vec<TrafficAircraft>), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_traffic(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<AtQuery>,
) -> Result<Json<Vec<TrafficAircraft>>, ApiError> {
    let p = pool(&state)?;
    let at = parse_at(&q)?;
    let data = reconstruct_at(p, at).await?;
    Ok(Json(flow_handlers::traffic_from(&data)))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/runway/{icao}",
    tag = "stats",
    params(
        ("icao" = String, Path, description = "Airport ICAO"),
        ("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")
    ),
    responses((status = 200, body = crate::feed::runway::RunwayBoard), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_runway(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(icao): Path<String>,
    Query(q): Query<AtQuery>,
) -> Result<Json<crate::feed::runway::RunwayBoard>, ApiError> {
    let p = pool(&state)?;
    let at = parse_at(&q)?;
    let icao = norm_icao(&icao);
    let data = reconstruct_at(p, at).await?;
    let winds = winds_for(p, at).await?;
    let snap = Arc::new(crate::feed::Snapshot::of(data));
    Ok(Json(
        runway_handlers::build_board_from(&state, &icao, snap, Arc::new(winds), at).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/taxi/{icao}",
    tag = "stats",
    params(
        ("icao" = String, Path, description = "Airport ICAO"),
        ("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")
    ),
    responses((status = 200, body = crate::feed::taxi::TaxiField), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_taxi(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Path(icao): Path<String>,
    Query(q): Query<AtQuery>,
) -> Result<Json<crate::feed::taxi::TaxiField>, ApiError> {
    let p = pool(&state)?;
    let at = parse_at(&q)?;
    let icao = norm_icao(&icao);
    // Taxi timing is replayed by feeding the stored position stream back through the live taxi
    // state machine (see reconstruct::taxi_field_at); needs the airport coordinate database.
    let airports = state.feed.read().await.airports.clone();
    Ok(Json(
        crate::feed::stats::reconstruct::taxi_field_at(p, airports.as_ref(), &icao, at).await?,
    ))
}

// --- historical traffic-management entities (active at instant T) ------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/fcas",
    tag = "stats",
    params(("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")),
    responses((status = 200, body = Vec<crate::models::FcaBody>), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_fcas(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<AtQuery>,
) -> Result<Json<Vec<crate::models::FcaBody>>, ApiError> {
    let p = pool(&state)?;
    Ok(Json(
        crate::repos::flow::list_fcas_at(p, parse_at(&q)?).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/tmis",
    tag = "stats",
    params(("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")),
    responses((status = 200, body = Vec<crate::models::TmiBody>), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_tmis(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<AtQuery>,
) -> Result<Json<Vec<crate::models::TmiBody>>, ApiError> {
    let p = pool(&state)?;
    Ok(Json(
        crate::repos::tmu::list_tmis_at(p, parse_at(&q)?).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/gdps",
    tag = "stats",
    params(("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")),
    responses((status = 200, body = Vec<crate::models::GdpBody>), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_gdps(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<AtQuery>,
) -> Result<Json<Vec<crate::models::GdpBody>>, ApiError> {
    let p = pool(&state)?;
    Ok(Json(
        crate::repos::gdp::list_gdps_at(p, parse_at(&q)?).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/hist/ground-stops",
    tag = "stats",
    params(("at" = i64, Query, description = "Reconstruct instant (Unix epoch seconds)")),
    responses((status = 200, body = Vec<crate::models::GroundStopBody>), (status = 400), (status = 401), (status = 503))
)]
pub async fn hist_ground_stops(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<AtQuery>,
) -> Result<Json<Vec<crate::models::GroundStopBody>>, ApiError> {
    let p = pool(&state)?;
    Ok(Json(
        crate::repos::tmu::list_ground_stops_at(p, parse_at(&q)?).await?,
    ))
}
