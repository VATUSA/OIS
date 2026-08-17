//! Read API over the persisted VATSIM stats (`/api/v1/stats/*`). Ported from the standalone stats
//! system's API, gated on `stats.read`. Historical lookups only — "now" is served by the live feed.

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
    models::{
        NetworkPointBody, StatsAirportBody, StatsFlightDetail, StatsFlightSummary, StatsTrackBody,
    },
    repos::stats as stats_repo,
    state::AppState,
};

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

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
