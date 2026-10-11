//! Read-only access to the stats history the aircraft-profile builder and its backtest mine (#828):
//! completed flights, their positions, the flight-plan revision in force at an instant, and the
//! winds snapshots around a flight. Nothing here writes.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{errors::ApiError, feed::winds::Winds};

/// A completed flight with a filed plan, as `stats.flight` stores it (the latest plan revision).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HistFlight {
    pub session_id: i64,
    pub departure: String,
    pub arrival: String,
    pub aircraft_short: Option<String>,
    pub aircraft_faa: Option<String>,
    pub cruise_tas: Option<i32>,
    pub cruise_alt: Option<i32>,
    pub route: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// One stored feed tick for a flight.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HistPosition {
    pub ts: DateTime<Utc>,
    pub lat: f32,
    pub lon: f32,
    pub altitude: i32,
    pub groundspeed: i16,
    pub heading: i16,
}

/// The parts of a flight-plan revision the backtest replays with.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HistPlan {
    pub route: Option<String>,
    pub cruise_alt: Option<i32>,
}

/// Completed flights with a departure, an arrival and an aircraft type, wholly inside
/// `[from, to)`, ordered by `session_id` so every run walks them in the same order.
pub async fn flights(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<HistFlight>, ApiError> {
    sqlx::query_as::<_, HistFlight>(
        "select session_id, departure, arrival, aircraft_short, aircraft_faa, cruise_tas, \
                cruise_alt, route, first_seen, last_seen \
         from stats.flight \
         where status = 'completed' and departure is not null and arrival is not null \
           and coalesce(aircraft_short, aircraft_faa) is not null \
           and first_seen >= $1 and last_seen < $2 \
         order by session_id",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Every stored position of one flight, oldest first.
pub async fn positions(pool: &PgPool, session_id: i64) -> Result<Vec<HistPosition>, ApiError> {
    sqlx::query_as::<_, HistPosition>(
        "select ts, lat, lon, altitude, groundspeed, heading from stats.position \
         where session_id = $1 order by ts",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// The flight-plan revision in force at `at` (the newest one that took effect at or before it), or
/// None when `stats.flight_plan` holds no revision for the flight yet.
pub async fn plan_at(
    pool: &PgPool,
    session_id: i64,
    at: DateTime<Utc>,
) -> Result<Option<HistPlan>, ApiError> {
    sqlx::query_as::<_, HistPlan>(
        "select route, cruise_alt from stats.flight_plan \
         where session_id = $1 and effective_from <= $2 \
         order by effective_from desc limit 1",
    )
    .bind(session_id)
    .bind(at)
    .fetch_optional(pool)
    .await
    .map_err(db)
}

/// The winds snapshots that cover `[from, to]`: every snapshot inside it plus the last one before
/// `from`, oldest first. A snapshot applies from its `ts` until the next one, the way
/// [`super::stats::winds_at`] picks one for replay.
pub async fn winds_covering(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<(DateTime<Utc>, Winds)>, ApiError> {
    let rows = sqlx::query_as::<_, (DateTime<Utc>, sqlx::types::Json<Winds>)>(
        "(select ts, data from stats.winds where ts < $1 order by ts desc limit 1) \
         union all \
         (select ts, data from stats.winds where ts >= $1 and ts <= $2) \
         order by ts",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().map(|(ts, w)| (ts, w.0)).collect())
}

fn db(e: sqlx::Error) -> ApiError {
    tracing::warn!(error = %e, "profile history db error");
    ApiError::Internal
}
