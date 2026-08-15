//! Persistence for VATUSA member sync — member detail on `identity.users`, the mirrored
//! roles/visits tables, and the per-facility webhook secrets.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::feed::vatusa::VatusaMember;
use crate::models::{VatusaProfile, VatusaRoleEntry};

/// The member's VATUSA details (for their profile / `/me`), or `None` if never synced.
pub async fn fetch_profile(pool: &PgPool, cid: i64) -> Result<Option<VatusaProfile>, ApiError> {
    type Row = (
        Option<String>,
        Option<i32>,
        Option<bool>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    );
    let row = sqlx::query_as::<_, Row>(
        "select home_facility, rating_numeric, flag_home_controller, facility_join, vatusa_synced_at
         from identity.users where cid = $1",
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    let Some((home_facility, rating_numeric, home_controller, facility_join, synced_at)) = row
    else {
        return Ok(None);
    };
    if synced_at.is_none() {
        return Ok(None); // user exists but hasn't been synced from VATUSA yet
    }

    let roles = sqlx::query_as::<_, (String, String)>(
        "select facility, role from identity.vatusa_roles where cid = $1 order by facility, role",
    )
    .bind(cid)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let visits = sqlx::query_scalar::<_, String>(
        "select facility from identity.vatusa_visits where cid = $1 order by facility",
    )
    .bind(cid)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(Some(VatusaProfile {
        home_facility,
        rating_numeric,
        home_controller,
        facility_join,
        synced_at,
        roles: roles
            .into_iter()
            .map(|(facility, role)| VatusaRoleEntry { facility, role })
            .collect(),
        visits,
    }))
}

/// Update a member's details and fully replace their roles/visits, keyed on CID. Only touches
/// users we already have (the row is created at login); a missing CID updates zero rows.
pub async fn upsert_member(pool: &PgPool, m: &VatusaMember) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;

    sqlx::query(
        "update identity.users set
             full_name = coalesce(nullif($2, ''), full_name),
             home_facility = nullif($3, ''),
             rating = coalesce($4, rating),
             rating_numeric = $5,
             flag_home_controller = $6,
             facility_join = $7,
             vatusa_synced_at = now(),
             updated_at = now()
         where cid = $1",
    )
    .bind(m.cid)
    .bind(m.full_name())
    .bind(&m.facility)
    .bind(m.short_rating())
    .bind(m.rating)
    .bind(m.flag_homecontroller)
    .bind(m.facility_join_ts())
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    sqlx::query("delete from identity.vatusa_roles where cid = $1")
        .bind(m.cid)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for role in &m.roles {
        if role.facility.is_empty() || role.role.is_empty() {
            continue;
        }
        let granted = role
            .created_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        sqlx::query(
            "insert into identity.vatusa_roles (cid, facility, role, granted_at)
             values ($1, $2, $3, $4)
             on conflict (cid, facility, role) do nothing",
        )
        .bind(m.cid)
        .bind(&role.facility)
        .bind(&role.role)
        .bind(granted)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }

    sqlx::query("delete from identity.vatusa_visits where cid = $1")
        .bind(m.cid)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for visit in &m.visiting_facilities {
        if visit.facility.is_empty() {
            continue;
        }
        sqlx::query(
            "insert into identity.vatusa_visits (cid, facility) values ($1, $2)
             on conflict (cid, facility) do nothing",
        )
        .bind(m.cid)
        .bind(&visit.facility)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }

    tx.commit().await.map_err(|_| ApiError::Internal)
}

/// CIDs from the given set that we actually have a user row for.
pub async fn known_cids(pool: &PgPool, cids: &[i64]) -> Result<Vec<i64>, ApiError> {
    if cids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar::<_, i64>("select cid from identity.users where cid = any($1)")
        .bind(cids)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// The least-recently-synced members (never-synced first), for reconciliation.
pub async fn stale_member_cids(pool: &PgPool, limit: i64) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar::<_, i64>(
        "select cid from identity.users
         where cid is not null
         order by vatusa_synced_at asc nulls first
         limit $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Active ARTCC ids, used to decide which facilities to register webhooks for.
pub async fn active_facilities(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select id from org.facilities where active = true order by id")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Facilities that already have a registered webhook.
pub async fn registered_facilities(pool: &PgPool) -> Result<HashSet<String>, ApiError> {
    let rows = sqlx::query_scalar::<_, String>("select facility from identity.vatusa_webhooks")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().collect())
}

pub async fn upsert_webhook(
    pool: &PgPool,
    facility: &str,
    webhook_id: i64,
    secret: &str,
    url: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into identity.vatusa_webhooks (facility, webhook_id, secret, url)
         values ($1, $2, $3, $4)
         on conflict (facility) do update
         set webhook_id = excluded.webhook_id,
             secret = excluded.secret,
             url = excluded.url,
             updated_at = now()",
    )
    .bind(facility)
    .bind(webhook_id)
    .bind(secret)
    .bind(url)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|_| ApiError::Internal)
}

/// The signing secret for a facility's webhook, used to verify inbound deliveries.
pub async fn webhook_secret(pool: &PgPool, facility: &str) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select secret from identity.vatusa_webhooks where facility = $1",
    )
    .bind(facility)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}
