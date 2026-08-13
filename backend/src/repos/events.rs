//! Event cache repo. The `events.event` table mirrors the upcoming VATUSA events the
//! sync job pulls in; per-event planning tables (added in later passes) reference it.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::models::{DccRequestBody, EventBody};

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
