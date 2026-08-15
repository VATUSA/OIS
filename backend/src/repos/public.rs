//! Read-only queries for the public advisories board (no auth). Each returns only
//! the currently-active rows, projected to the lean public DTOs — active being:
//! restrictions/ground stops = published & not past their end; GDPs = published;
//! programs = live (not past `active_until`); FCAs = enabled.

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{PublicGdp, PublicGroundStop, PublicProgram, PublicRestriction},
};

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
    sqlx::query_as::<_, PublicGdp>(
        "select id, airport, aar, scope, start_time, end_time, max_enroute_min, exempt_airborne \
         from tmu.gdp \
         where status = 'published' \
         order by airport",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn active_programs(pool: &PgPool) -> Result<Vec<PublicProgram>, ApiError> {
    sqlx::query_as::<_, PublicProgram>(
        "select icao, aar, trail, mit, gates, exclude_wake, exclude_types, jets_only, active_until \
         from tmu.programs \
         where active_until is null or active_until > now() \
         order by icao",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}
