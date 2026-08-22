//! ACE support: the shared request queue (open → claimed → completed/cancelled) and the team roster.
//! State transitions are guarded inside a transaction (`select … for update`), per OIS convention —
//! the coarse `RequirePermission` gate authorizes the caller; the state check lives here.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{AceRequestBody, AceTeamMemberBody},
};

const REQUEST_SELECT: &str = "select r.id, \
    ru.cid as requested_by_cid, ru.display_name as requested_by_name, \
    r.artcc_id, r.position, r.requested_for, r.details, r.status, \
    cu.display_name as claimed_by_name, r.claimed_at, \
    du.display_name as decided_by_name, r.decided_at, r.created_at \
    from ace.requests r \
    join identity.users ru on ru.id = r.requested_by \
    left join identity.users cu on cu.id = r.claimed_by \
    left join identity.users du on du.id = r.decided_by";

/// The queue, optionally filtered by status and/or ARTCC, newest first.
pub async fn list_requests(
    pool: &PgPool,
    status: Option<&str>,
    artcc_id: Option<&str>,
) -> Result<Vec<AceRequestBody>, ApiError> {
    sqlx::query_as::<_, AceRequestBody>(&format!(
        "{REQUEST_SELECT} \
         where ($1::text is null or r.status = $1) \
           and ($2::text is null or r.artcc_id = $2) \
         order by r.created_at desc"
    ))
    .bind(status)
    .bind(artcc_id)
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

/// Open a new request. Returns the new id.
#[allow(clippy::too_many_arguments)]
pub async fn create_request(
    pool: &PgPool,
    requested_by: &str,
    artcc_id: Option<&str>,
    position: Option<&str>,
    requested_for: Option<DateTime<Utc>>,
    details: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into ace.requests (requested_by, artcc_id, position, requested_for, details) \
         values ($1, $2, $3, $4, $5) returning id",
    )
    .bind(requested_by)
    .bind(artcc_id)
    .bind(position)
    .bind(requested_for)
    .bind(details)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Claim an `open` request. `NotFound` if absent, `Conflict` if not open (data-dependent, in-tx).
pub async fn claim_request(pool: &PgPool, id: &str, claimer: &str) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let status =
        sqlx::query_scalar::<_, String>("select status from ace.requests where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
    match status.as_deref() {
        None => return Err(ApiError::NotFound),
        Some("open") => {}
        Some(_) => return Err(ApiError::Conflict),
    }
    sqlx::query(
        "update ace.requests set status = 'claimed', claimed_by = $2, claimed_at = now() \
         where id = $1",
    )
    .bind(id)
    .bind(claimer)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
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

// --- team roster ---

/// The ACE roster (active first, then by name). `active_only` hides soft-removed members.
pub async fn list_team(
    pool: &PgPool,
    active_only: bool,
) -> Result<Vec<AceTeamMemberBody>, ApiError> {
    let sql = format!(
        "select t.id, u.cid, u.display_name, t.role, t.artcc_id, t.active \
         from ace.team_members t join identity.users u on u.id = t.user_id \
         {} order by t.active desc, u.display_name",
        if active_only { "where t.active" } else { "" }
    );
    sqlx::query_as::<_, AceTeamMemberBody>(&sql)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Add or update a roster member (keyed on user), returning its id.
pub async fn upsert_team_member(
    pool: &PgPool,
    user_id: &str,
    role: Option<&str>,
    artcc_id: Option<&str>,
    active: bool,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into ace.team_members (user_id, role, artcc_id, active) \
         values ($1, $2, $3, $4) \
         on conflict (user_id) do update \
             set role = excluded.role, artcc_id = excluded.artcc_id, active = excluded.active",
    )
    .bind(user_id)
    .bind(role)
    .bind(artcc_id)
    .bind(active)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Permanently remove a roster member. Returns false if absent.
pub async fn remove_team_member(pool: &PgPool, user_id: &str) -> Result<bool, ApiError> {
    let res = sqlx::query("delete from ace.team_members where user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(res.rows_affected() > 0)
}
