//! Event cache repo. The `events.event` table mirrors the upcoming VATUSA events the
//! sync job pulls in; per-event planning tables (added in later passes) reference it.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::models::{
    AirportRateBody, DccRequestBody, EventBody, FacilitySupportBody, StaffingRequestBody,
};

const EVENT_SELECT: &str = "select id, title, body, banner_image_url, facility, \
    start_time, end_time, review_status from events.event";

/// Upcoming (and in-progress) events, soonest first.
pub async fn list_upcoming(pool: &PgPool) -> Result<Vec<EventBody>, ApiError> {
    sqlx::query_as::<_, EventBody>(&format!(
        "{EVENT_SELECT} where end_time >= now() order by start_time"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get(pool: &PgPool, id: i64) -> Result<Option<EventBody>, ApiError> {
    sqlx::query_as::<_, EventBody>(&format!("{EVENT_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Upsert a synced batch of events (VATUSA id is the primary key).
pub async fn upsert_many(pool: &PgPool, events: &[EventBody]) -> Result<(), ApiError> {
    for e in events {
        sqlx::query(
            "insert into events.event
                 (id, title, body, banner_image_url, facility, start_time, end_time,
                  review_status, synced_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, now())
             on conflict (id) do update set
                 title = excluded.title,
                 body = excluded.body,
                 banner_image_url = excluded.banner_image_url,
                 facility = excluded.facility,
                 start_time = excluded.start_time,
                 end_time = excluded.end_time,
                 review_status = excluded.review_status,
                 synced_at = now()",
        )
        .bind(e.id)
        .bind(&e.title)
        .bind(&e.body)
        .bind(&e.banner_image_url)
        .bind(&e.facility)
        .bind(e.start_time)
        .bind(e.end_time)
        .bind(&e.review_status)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    Ok(())
}

/// Drop cached events that ended before `cutoff` (housekeeping for the sync job).
pub async fn prune(pool: &PgPool, cutoff: DateTime<Utc>) -> Result<u64, ApiError> {
    let result = sqlx::query("delete from events.event where end_time < $1")
        .bind(cutoff)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected())
}

// --- DCC support ---

const DCC_SELECT: &str = "select d.status, d.notes, d.updated_at, u.display_name as updated_by \
    from events.dcc_request d left join identity.users u on u.id = d.updated_by";

pub async fn get_dcc(pool: &PgPool, event_id: i64) -> Result<Option<DccRequestBody>, ApiError> {
    sqlx::query_as::<_, DccRequestBody>(&format!("{DCC_SELECT} where d.event_id = $1"))
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn upsert_dcc(
    pool: &PgPool,
    event_id: i64,
    status: &str,
    notes: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.dcc_request (event_id, status, notes, updated_by)
         values ($1, $2, $3, $4)
         on conflict (event_id) do update set
             status = excluded.status,
             notes = excluded.notes,
             updated_by = excluded.updated_by",
    )
    .bind(event_id)
    .bind(status)
    .bind(notes)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

// --- facility support matrix ---

const FS_SELECT: &str = "select f.facility, f.level, f.notes, f.updated_at, \
    u.display_name as updated_by \
    from events.facility_support f left join identity.users u on u.id = f.updated_by";

pub async fn list_facility_support(
    pool: &PgPool,
    event_id: i64,
) -> Result<Vec<FacilitySupportBody>, ApiError> {
    sqlx::query_as::<_, FacilitySupportBody>(&format!(
        "{FS_SELECT} where f.event_id = $1 order by f.facility"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_facility_support(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
) -> Result<Option<FacilitySupportBody>, ApiError> {
    sqlx::query_as::<_, FacilitySupportBody>(&format!(
        "{FS_SELECT} where f.event_id = $1 and f.facility = $2"
    ))
    .bind(event_id)
    .bind(facility)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn upsert_facility_support(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
    level: &str,
    notes: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.facility_support (event_id, facility, level, notes, updated_by)
         values ($1, $2, $3, $4, $5)
         on conflict (event_id, facility) do update set
             level = excluded.level,
             notes = excluded.notes,
             updated_by = excluded.updated_by",
    )
    .bind(event_id)
    .bind(facility)
    .bind(level)
    .bind(notes)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn delete_facility_support(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
) -> Result<bool, ApiError> {
    let result =
        sqlx::query("delete from events.facility_support where event_id = $1 and facility = $2")
            .bind(event_id)
            .bind(facility)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- airport rates (AAR/ADR) ---

const RATE_SELECT: &str = "select r.icao, r.aar, r.adr, r.artcc, r.updated_at, \
    u.display_name as updated_by \
    from events.airport_rate r left join identity.users u on u.id = r.updated_by";

pub async fn list_airport_rates(
    pool: &PgPool,
    event_id: i64,
) -> Result<Vec<AirportRateBody>, ApiError> {
    sqlx::query_as::<_, AirportRateBody>(&format!(
        "{RATE_SELECT} where r.event_id = $1 order by r.icao"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_airport_rate(
    pool: &PgPool,
    event_id: i64,
    icao: &str,
) -> Result<Option<AirportRateBody>, ApiError> {
    sqlx::query_as::<_, AirportRateBody>(&format!(
        "{RATE_SELECT} where r.event_id = $1 and r.icao = $2"
    ))
    .bind(event_id)
    .bind(icao)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn upsert_airport_rate(
    pool: &PgPool,
    event_id: i64,
    icao: &str,
    aar: i32,
    adr: i32,
    artcc: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.airport_rate (event_id, icao, aar, adr, artcc, updated_by)
         values ($1, $2, $3, $4, $5, $6)
         on conflict (event_id, icao) do update set
             aar = excluded.aar,
             adr = excluded.adr,
             artcc = excluded.artcc,
             updated_by = excluded.updated_by",
    )
    .bind(event_id)
    .bind(icao)
    .bind(aar)
    .bind(adr)
    .bind(artcc)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn delete_airport_rate(
    pool: &PgPool,
    event_id: i64,
    icao: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from events.airport_rate where event_id = $1 and icao = $2")
        .bind(event_id)
        .bind(icao)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- ACE staffing requests ---

const STAFFING_SELECT: &str = "select s.facility, s.positions_requested, s.positions_filled, \
    s.status, s.notes, s.updated_at, u.display_name as updated_by \
    from events.staffing_request s left join identity.users u on u.id = s.updated_by";

pub async fn list_staffing(
    pool: &PgPool,
    event_id: i64,
) -> Result<Vec<StaffingRequestBody>, ApiError> {
    sqlx::query_as::<_, StaffingRequestBody>(&format!(
        "{STAFFING_SELECT} where s.event_id = $1 order by s.facility"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_staffing(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
) -> Result<Option<StaffingRequestBody>, ApiError> {
    sqlx::query_as::<_, StaffingRequestBody>(&format!(
        "{STAFFING_SELECT} where s.event_id = $1 and s.facility = $2"
    ))
    .bind(event_id)
    .bind(facility)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_staffing(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
    requested: i32,
    filled: i32,
    status: &str,
    notes: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.staffing_request
             (event_id, facility, positions_requested, positions_filled, status, notes, updated_by)
         values ($1, $2, $3, $4, $5, $6, $7)
         on conflict (event_id, facility) do update set
             positions_requested = excluded.positions_requested,
             positions_filled = excluded.positions_filled,
             status = excluded.status,
             notes = excluded.notes,
             updated_by = excluded.updated_by",
    )
    .bind(event_id)
    .bind(facility)
    .bind(requested)
    .bind(filled)
    .bind(status)
    .bind(notes)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn delete_staffing(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
) -> Result<bool, ApiError> {
    let result =
        sqlx::query("delete from events.staffing_request where event_id = $1 and facility = $2")
            .bind(event_id)
            .bind(facility)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}
