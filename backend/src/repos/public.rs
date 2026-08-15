//! Read-only queries for the public advisories board (no auth). Each returns only
//! the currently-active rows, projected to the lean public DTOs — active being:
//! restrictions/ground stops = published & not past their end; GDPs = published;
//! programs = live (not past `active_until`); FCAs = enabled.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{PublicGdp, PublicGroundStop, PublicProgram, PublicRestriction},
};

/// A published GDP for one arrival airport: (id, aar, start HHMM, end HHMM).
pub async fn gdp_for_airport(
    pool: &PgPool,
    airport: &str,
) -> Result<Option<(String, i32, String, String)>, ApiError> {
    sqlx::query_as::<_, (String, i32, String, String)>(
        "select id, aar, start_time, end_time from tmu.gdp \
         where airport = $1 and status = 'published' order by updated_at desc limit 1",
    )
    .bind(airport)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// A flight's frozen GDP control slot: (edct, cta, delay_min).
pub async fn gdp_slot_for(
    pool: &PgPool,
    gdp_id: &str,
    callsign: &str,
) -> Result<Option<(Option<DateTime<Utc>>, DateTime<Utc>, i32)>, ApiError> {
    sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>, i32)>(
        "select edct, cta, delay_min from tmu.gdp_slot where gdp_id = $1 and callsign = $2",
    )
    .bind(gdp_id)
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// An active ground stop for one arrival airport: (scope, until).
pub async fn ground_stop_for_airport(
    pool: &PgPool,
    airport: &str,
) -> Result<Option<(String, Option<String>)>, ApiError> {
    sqlx::query_as::<_, (String, Option<String>)>(
        "select scope, until from tmu.ground_stops \
         where airport = $1 and status = 'published' \
           and (tmu.ground_stop_until_ts(created_at, until) is null \
                or tmu.ground_stop_until_ts(created_at, until) > now()) \
         order by updated_at desc limit 1",
    )
    .bind(airport)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn active_restrictions(pool: &PgPool) -> Result<Vec<PublicRestriction>, ApiError> {
    sqlx::query_as::<_, PublicRestriction>(
        "select id, requesting, providing, restriction, start_time, stop_time \
         from tmu.tmis \
         where status = 'published' and (stop_time is null or stop_time > now()) \
         order by start_time",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn active_ground_stops(pool: &PgPool) -> Result<Vec<PublicGroundStop>, ApiError> {
    sqlx::query_as::<_, PublicGroundStop>(
        "select id, airport, scope, until \
         from tmu.ground_stops \
         where status = 'published' \
           and (tmu.ground_stop_until_ts(created_at, until) is null \
                or tmu.ground_stop_until_ts(created_at, until) > now()) \
         order by airport",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn active_gdps(pool: &PgPool) -> Result<Vec<PublicGdp>, ApiError> {
    // Delay stats come from the frozen control slots; demand_60min/over_capacity
    // are placeholders the handler fills from the live feed.
    sqlx::query_as::<_, PublicGdp>(
        "select g.id, g.airport, g.aar, g.scope, g.start_time, g.end_time, \
                g.max_enroute_min, g.exempt_airborne, \
                coalesce(s.controlled, 0::bigint) as controlled, \
                coalesce(s.avg_delay, 0::bigint) as avg_delay_min, \
                coalesce(s.max_delay, 0::bigint) as max_delay_min, \
                0::bigint as demand_60min, false as over_capacity \
         from tmu.gdp g \
         left join ( \
             select gdp_id, count(*)::bigint as controlled, \
                    round(avg(delay_min))::bigint as avg_delay, \
                    max(delay_min)::bigint as max_delay \
             from tmu.gdp_slot group by gdp_id \
         ) s on s.gdp_id = g.id \
         where g.status = 'published' \
         order by g.airport",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn active_programs(pool: &PgPool) -> Result<Vec<PublicProgram>, ApiError> {
    sqlx::query_as::<_, PublicProgram>(
        "select icao, aar, trail, mit, gates, exclude_wake, exclude_types, jets_only, active_until, \
                0::bigint as demand_60min, false as over_capacity \
         from tmu.programs \
         where active_until is null or active_until > now() \
         order by icao",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}
