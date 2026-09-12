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

/// A claim whose event start has crossed a reminder threshold, with what the DM needs.
#[derive(sqlx::FromRow)]
pub struct DueReminder {
    pub claim_id: String,
    pub discord_user_id: String,
    pub event_title: String,
    pub position: Option<String>,
}

/// Claims due for a `job_type` reminder: their event starts within the next `hours_before` hours
/// (i.e. the T-`hours_before`h threshold has just been crossed and the event hasn't started yet),
/// the claimer has a linked Discord account, the request isn't cancelled, and no `job_type` job has
/// already been enqueued for this claim — so a released claim or a cancelled request naturally
/// drops out, and a repeat scheduler tick never double-sends.
pub async fn claims_due_for_reminder(
    pool: &PgPool,
    hours_before: i64,
    job_type: &str,
) -> Result<Vec<DueReminder>, ApiError> {
    sqlx::query_as::<_, DueReminder>(
        "select c.id as claim_id, m.external_id as discord_user_id, e.title as event_title, \
                r.position \
         from ace.claims c \
         join ace.requests r on r.id = c.request_id and r.status <> 'cancelled' \
         join events.event e on e.id = r.event_id \
         join integration.external_sync_mappings m \
           on m.system_code = 'discord' and m.entity_type = 'user' and m.local_id = c.claimed_by \
         where e.start_time > now() \
           and e.start_time <= now() + make_interval(hours => $1::int) \
           and not exists ( \
             select 1 from integration.outbound_jobs j \
             where j.job_type = $2 and j.subject_type = 'ace_claim' and j.subject_id = c.id \
           )",
    )
    .bind(hours_before as i32)
    .bind(job_type)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    async fn seed_user(pool: &PgPool, name: &str) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into identity.users (full_name, display_name) values ($1, $1) returning id",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn seed_event(pool: &PgPool, id: i64, hours_from_now: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "insert into events.event (id, title, start_time, end_time) \
             values ($1, 'Fall Fly-In', now() + make_interval(hours => $2::int), \
                     now() + make_interval(hours => $2::int) + interval '2 hours') \
             returning id",
        )
        .bind(id)
        .bind(hours_from_now as i32)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn link_discord(pool: &PgPool, user_id: &str, discord_id: &str) {
        sqlx::query(
            "insert into integration.external_sync_mappings \
             (system_code, entity_type, local_id, external_id) \
             values ('discord', 'user', $1, $2)",
        )
        .bind(user_id)
        .bind(discord_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_claimed_request(
        pool: &PgPool,
        event_id: i64,
        requester: &str,
        claimer: &str,
    ) -> String {
        let mut tx = pool.begin().await.unwrap();
        let request_id = create_request(
            &mut tx,
            event_id,
            requester,
            Some("ZDC"),
            Some("DCA_APP"),
            1,
            "need coverage",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        claim_request(&mut tx, &request_id, claimer, "", None, None)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        request_id
    }

    async fn mark_job_sent(pool: &PgPool, job_type: &str, claim_id: &str) {
        sqlx::query(
            "insert into integration.outbound_jobs (job_type, payload, subject_type, subject_id) \
             values ($1, '{}'::jsonb, 'ace_claim', $2)",
        )
        .bind(job_type)
        .bind(claim_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn claim_id_for(pool: &PgPool, request_id: &str) -> String {
        sqlx::query_scalar::<_, String>("select id from ace.claims where request_id = $1")
            .bind(request_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn claims_due_for_reminder_matches_the_24h_scheduling_math(pool: PgPool) {
        let requester = seed_user(&pool, "Requester").await;
        let claimer = seed_user(&pool, "Claimer").await;
        link_discord(&pool, &claimer, "999888777").await;

        let due_event = seed_event(&pool, 1, 23).await; // inside the T-24h window
        let not_due_event = seed_event(&pool, 2, 30).await; // outside it

        let due_request = seed_claimed_request(&pool, due_event, &requester, &claimer).await;
        seed_claimed_request(&pool, not_due_event, &requester, &claimer).await;

        let due = claims_due_for_reminder(&pool, 24, "ace_claim_reminder_24h")
            .await
            .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].discord_user_id, "999888777");
        assert_eq!(due[0].event_title, "Fall Fly-In");
        assert_eq!(due[0].position.as_deref(), Some("DCA_APP"));
        assert_eq!(due[0].claim_id, claim_id_for(&pool, &due_request).await);
    }

    #[sqlx::test]
    async fn claims_due_for_reminder_excludes_a_cancelled_request(pool: PgPool) {
        let requester = seed_user(&pool, "Requester").await;
        let claimer = seed_user(&pool, "Claimer").await;
        link_discord(&pool, &claimer, "999888777").await;
        let event_id = seed_event(&pool, 1, 5).await; // inside the T-6h window too

        let request_id = seed_claimed_request(&pool, event_id, &requester, &claimer).await;
        decide_request(&pool, &request_id, &requester, "cancelled")
            .await
            .unwrap();

        let due = claims_due_for_reminder(&pool, 6, "ace_claim_reminder_6h")
            .await
            .unwrap();
        assert!(due.is_empty());
    }

    #[sqlx::test]
    async fn claims_due_for_reminder_is_idempotent(pool: PgPool) {
        let requester = seed_user(&pool, "Requester").await;
        let claimer = seed_user(&pool, "Claimer").await;
        link_discord(&pool, &claimer, "999888777").await;
        let event_id = seed_event(&pool, 1, 23).await;
        let request_id = seed_claimed_request(&pool, event_id, &requester, &claimer).await;
        let claim_id = claim_id_for(&pool, &request_id).await;

        mark_job_sent(&pool, "ace_claim_reminder_24h", &claim_id).await;

        let due = claims_due_for_reminder(&pool, 24, "ace_claim_reminder_24h")
            .await
            .unwrap();
        assert!(due.is_empty(), "already-reminded claim must not resurface");
    }
}
