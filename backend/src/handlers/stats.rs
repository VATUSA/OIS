//! Read API over the persisted VATSIM stats (`/api/v1/stats/*`). Ported from the standalone stats
//! system's API, gated on `stats.read`. Historical lookups only — "now" is served by the live feed.

use std::collections::HashMap;

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
        AtcBoard, CaptureSummaryBody, DeparturesResponse, NetworkPointBody, ReplayBody,
        ReplayFlightBody, StatsAirportBody, StatsFlightDetail, StatsFlightSummary, StatsTrackBody,
        TrafficAircraft,
    },
    repos::stats as stats_repo,
    state::AppState,
};

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

/// (callsign, departure, arrival, aircraft) for a replay flight.
type FlightMeta = (String, Option<String>, Option<String>, Option<String>);

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
    stats_repo::flight_detail(pool(&state)?, parse_session_id(&id)?)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
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

    let samples = stats_repo::replay_positions(p, from, to, step).await?;

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
            departure: None,
            arrival: None,
            aircraft: None,
            samples: track,
        });
    }

    // Attach callsign + plan basics.
    let meta: HashMap<i64, FlightMeta> = stats_repo::flights_meta(p, &ids)
        .await?
        .into_iter()
        .map(|(sid, cs, dep, arr, ac)| (sid, (cs, dep, arr, ac)))
        .collect();
    for f in flights.iter_mut() {
        if let Some((cs, dep, arr, ac)) = meta.get(&f.session_id) {
            f.callsign = cs.clone();
            f.departure = dep.clone();
            f.arrival = arr.clone();
            f.aircraft = ac.clone();
        }
    }

    Ok(Json(ReplayBody {
        capture_id: cap.id,
        window_start: from,
        window_end: to,
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
    Ok(Json(
        feed_handlers::flow_from_data(&state, p, &icao, &data, at).await?,
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
    Ok(Json(
        feed_handlers::departures_response(&state, p, &dep, &data, at).await?,
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
    Ok(Json(
        runway_handlers::build_board_from(&state, &icao, &data, at).await?,
    ))
}
