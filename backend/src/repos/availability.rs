//! DCC event-thread availability: one response per person (latest press wins), surfaced in the
//! event planner. Written by the Discord callback; read by the planner panel.

use sqlx::PgPool;

use crate::{errors::ApiError, models::EventAvailabilityBody};

/// Record (or replace) a person's availability for an event. One row per `(event, user)`.
pub async fn set_availability(
    pool: &PgPool,
    event_id: i64,
    user_id: &str,
    status: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.availability (event_id, user_id, status, updated_at) \
         values ($1, $2, $3, now()) \
         on conflict (event_id, user_id) \
         do update set status = excluded.status, updated_at = now()",
    )
    .bind(event_id)
    .bind(user_id)
    .bind(status)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Everyone who has indicated availability for an event, with their assignable roles for badging.
/// Ordered by status then name so the panel groups cleanly.
pub async fn list_for_event(
    pool: &PgPool,
    event_id: i64,
) -> Result<Vec<EventAvailabilityBody>, ApiError> {
    sqlx::query_as::<_, EventAvailabilityBody>(
        "select u.cid, u.display_name, a.status, \
            coalesce((select json_agg(ur.role_name order by ur.role_name) \
              from access.user_roles ur where ur.user_id = u.id), '[]'::json) as roles, \
            a.updated_at \
         from events.availability a \
         join identity.users u on u.id = a.user_id \
         where a.event_id = $1 \
         order by a.status, u.display_name",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}
