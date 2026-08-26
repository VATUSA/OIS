//! ACE support: the event-scoped request queue (open → completed/cancelled) + per-person slot claims.
//! State transitions are guarded inside a transaction (`select … for update`), per OIS convention —
//! the coarse `RequirePermission` gate authorizes the caller; the state check lives here.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    errors::ApiError,
    models::{AceClaimBody, AceRequestBody},
};

/// The request row + its aggregated claims (json_agg) + a live claim count. `slots` and the count
/// let the client derive "filled". Column order must match `AceRequestBody` (FromRow).
const REQUEST_SELECT: &str = "select r.id, r.event_id, \
    ru.cid as requested_by_cid, ru.display_name as requested_by_name, \
    r.artcc_id, r.position, r.slots, r.details, r.status, \
    coalesce((select json_agg(json_build_object( \
        'cid', cu.cid, 'display_name', cu.display_name, 'notes', cl.notes, \
        'start_time', cl.start_time, 'end_time', cl.end_time, 'claimed_at', cl.claimed_at) \
        order by cl.claimed_at) \
      from ace.claims cl join identity.users cu on cu.id = cl.claimed_by \
      where cl.request_id = r.id), '[]'::json) as claims, \
    (select count(*) from ace.claims cl where cl.request_id = r.id) as claims_count, \
    du.display_name as decided_by_name, r.decided_at, r.created_at \
    from ace.requests r \
    join identity.users ru on ru.id = r.requested_by \
    left join identity.users du on du.id = r.decided_by";

/// An event's ACE requests, optionally filtered by status, newest first.
pub async fn list_requests(
    pool: &PgPool,
    event_id: i64,
    status: Option<&str>,
) -> Result<Vec<AceRequestBody>, ApiError> {
    sqlx::query_as::<_, AceRequestBody>(&format!(
        "{REQUEST_SELECT} \
         where r.event_id = $1 and ($2::text is null or r.status = $2) \
         order by r.created_at desc"
    ))
    .bind(event_id)
    .bind(status)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_request(pool: &PgPool, id: &str) -> Result<Option<AceRequestBody>, ApiError> {
    sqlx::query_as::<_, AceRequestBody>(&format!("{REQUEST_SELECT} where r.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Distinct ARTCCs with an open ACE request on the event — surfaces the `has_staffing` signal on the
/// event's facility-support list (replaces the old staffing-request source).
pub async fn open_request_artccs(pool: &PgPool, event_id: i64) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select distinct artcc_id from ace.requests \
         where event_id = $1 and status = 'open' and artcc_id is not null",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Open a new request **in the caller's transaction** (so a Discord post-job can be enqueued
/// atomically). Returns the new id.
#[allow(clippy::too_many_arguments)]
pub async fn create_request(
    tx: &mut Transaction<'_, Postgres>,
    event_id: i64,
    requested_by: &str,
    artcc_id: Option<&str>,
    position: Option<&str>,
    slots: i32,
    details: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into ace.requests (event_id, requested_by, artcc_id, position, slots, details) \
         values ($1, $2, $3, $4, $5, $6) returning id",
    )
    .bind(event_id)
    .bind(requested_by)
    .bind(artcc_id)
    .bind(position)
    .bind(slots)
    .bind(details)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Claim a slot on an `open` request **in the caller's transaction**. Guards (all `Conflict`): request
/// not open, already full (`claims >= slots`), or the caller already holds a claim (unique). Returns
/// `(slots, claims_count_after)` for the notify job. `NotFound` if the request is absent.
pub async fn claim_request(
    tx: &mut Transaction<'_, Postgres>,
    request_id: &str,
    claimer: &str,
    notes: &str,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
) -> Result<(i32, i64), ApiError> {
    let row = sqlx::query_as::<_, (String, i32)>(
        "select status, slots from ace.requests where id = $1 for update",
    )
    .bind(request_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    let (status, slots) = row.ok_or(ApiError::NotFound)?;
    if status != "open" {
        return Err(ApiError::Conflict);
    }
    let count: i64 = sqlx::query_scalar("select count(*) from ace.claims where request_id = $1")
        .bind(request_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    if count >= slots as i64 {
        return Err(ApiError::Conflict); // full
    }
    let inserted = sqlx::query(
        "insert into ace.claims (request_id, claimed_by, notes, start_time, end_time) \
         values ($1, $2, $3, $4, $5) on conflict (request_id, claimed_by) do nothing",
    )
    .bind(request_id)
    .bind(claimer)
    .bind(notes)
    .bind(start_time)
    .bind(end_time)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Conflict); // already claimed by this user
    }
    Ok((slots, count + 1))
}

/// Release the caller's own claim **in the caller's transaction**. Returns `(slots, claims_count_after)`
/// for the notify job; `NotFound` if the caller had no claim on this request.
pub async fn release_claim(
    tx: &mut Transaction<'_, Postgres>,
    request_id: &str,
    claimer: &str,
) -> Result<(i32, i64), ApiError> {
    let deleted = sqlx::query("delete from ace.claims where request_id = $1 and claimed_by = $2")
        .bind(request_id)
        .bind(claimer)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query_as::<_, (i32, i64)>(
        "select r.slots, (select count(*) from ace.claims where request_id = r.id) \
         from ace.requests r where r.id = $1",
    )
    .bind(request_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The current claims on a request, read **inside a transaction** (so a notify job can be built from
/// the post-claim state before commit).
pub async fn claims_for(
    tx: &mut Transaction<'_, Postgres>,
    request_id: &str,
) -> Result<Vec<AceClaimBody>, ApiError> {
    sqlx::query_as::<_, AceClaimBody>(
        "select cu.cid, cu.display_name, cl.notes, cl.start_time, cl.end_time, cl.claimed_at \
         from ace.claims cl join identity.users cu on cu.id = cl.claimed_by \
         where cl.request_id = $1 order by cl.claimed_at",
    )
    .bind(request_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Permanently delete a request (and its claims, via FK cascade). Returns false if absent.
pub async fn delete_request(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let res = sqlx::query("delete from ace.requests where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(res.rows_affected() > 0)
}

/// Complete or cancel a request. `NotFound` if absent, `Conflict` if already terminal. `outcome`
/// must be `completed` or `cancelled` (validated by the caller).
pub async fn decide_request(
    pool: &PgPool,
    id: &str,
    decider: &str,
    outcome: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let status =
        sqlx::query_scalar::<_, String>("select status from ace.requests where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
    match status.as_deref() {
        None => return Err(ApiError::NotFound),
        Some("completed") | Some("cancelled") => return Err(ApiError::Conflict),
        Some(_) => {}
    }
    sqlx::query(
        "update ace.requests set status = $2, decided_by = $3, decided_at = now() where id = $1",
    )
    .bind(id)
    .bind(outcome)
    .bind(decider)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}
