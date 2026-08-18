//! Ground Delay Program persistence — the program (lifecycle like ground stops) plus its
//! frozen control-time slots (written at publish so issued EDCTs don't drift).

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{AarStep, GdpBody},
};

const GDP_SELECT: &str = "select g.id, g.airport, g.aar, g.scope, g.start_time, g.end_time, \
    g.max_enroute_min, g.exempt_airborne, g.aar_steps, g.status, g.published_at, g.updated_at, \
    u.display_name as updated_by \
    from tmu.gdp g left join identity.users u on u.id = g.updated_by";

pub async fn list_gdps(pool: &PgPool) -> Result<Vec<GdpBody>, ApiError> {
    sqlx::query_as::<_, GdpBody>(&format!("{GDP_SELECT} order by g.updated_at desc"))
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn get_gdp(pool: &PgPool, id: &str) -> Result<Option<GdpBody>, ApiError> {
    sqlx::query_as::<_, GdpBody>(&format!("{GDP_SELECT} where g.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_gdp(
    pool: &PgPool,
    airport: &str,
    aar: i32,
    scope: &str,
    start_time: &str,
    end_time: &str,
    max_enroute_min: Option<i32>,
    exempt_airborne: bool,
    aar_steps: &[AarStep],
    actor: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into tmu.gdp \
           (airport, aar, scope, start_time, end_time, max_enroute_min, exempt_airborne, \
            aar_steps, created_by, updated_by) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) returning id",
    )
    .bind(airport)
    .bind(aar)
    .bind(scope)
    .bind(start_time)
    .bind(end_time)
    .bind(max_enroute_min)
    .bind(exempt_airborne)
    .bind(sqlx::types::Json(aar_steps))
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Revise a GDP's mutable fields (airport is immutable). Only draft/published programs
/// are editable. Returns false when the id is absent or terminal.
#[allow(clippy::too_many_arguments)]
pub async fn update_gdp(
    pool: &PgPool,
    id: &str,
    aar: i32,
    scope: &str,
    start_time: &str,
    end_time: &str,
    max_enroute_min: Option<i32>,
    exempt_airborne: bool,
    aar_steps: &[AarStep],
    actor: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.gdp set \
            aar = $2, scope = $3, start_time = $4, end_time = $5, \
            max_enroute_min = $6, exempt_airborne = $7, aar_steps = $8, updated_by = $9 \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .bind(aar)
    .bind(scope)
    .bind(start_time)
    .bind(end_time)
    .bind(max_enroute_min)
    .bind(exempt_airborne)
    .bind(sqlx::types::Json(aar_steps))
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Publish a draft GDP. Returns false if it isn't currently a draft.
pub async fn publish_gdp(pool: &PgPool, id: &str, published_by: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.gdp set status = 'published', published_by = $2, published_at = now() \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(published_by)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Cancel a draft or published GDP. Returns false if it's already terminal.
pub async fn cancel_gdp(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.gdp set status = 'cancelled', ended_at = coalesce(ended_at, now()) \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_gdp(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    // Never-published drafts vanish (slots cascade); published GDPs are kept for replay.
    crate::repos::tmu::delete_or_retain(pool, "tmu.gdp", id).await
}

/// GDPs that were live at instant `at` (for historical replay).
pub async fn list_gdps_at(pool: &PgPool, at: DateTime<Utc>) -> Result<Vec<GdpBody>, ApiError> {
    const GDP_END: &str =
        "tmu.gdp_end_ts(coalesce(g.published_at, g.created_at), g.start_time, g.end_time)";
    sqlx::query_as::<_, GdpBody>(&format!(
        "{GDP_SELECT} where g.published_at is not null and g.published_at <= $1 \
           and ({GDP_END} is null or {GDP_END} > $1) \
           and (g.ended_at is null or g.ended_at > $1) \
         order by g.updated_at desc"
    ))
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

// --- frozen control-time slots ---

/// One frozen control time assigned at publish.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GdpSlotRow {
    pub callsign: String,
    pub dep: String,
    pub original_eta: DateTime<Utc>,
    pub cta: DateTime<Utc>,
    pub edct: Option<DateTime<Utc>>,
    pub delay_min: i32,
}

pub async fn list_slots(pool: &PgPool, gdp_id: &str) -> Result<Vec<GdpSlotRow>, ApiError> {
    sqlx::query_as::<_, GdpSlotRow>(
        "select callsign, dep, original_eta, cta, edct, delay_min \
         from tmu.gdp_slot where gdp_id = $1",
    )
    .bind(gdp_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Freeze a fresh set of control times for `gdp_id`, replacing any previous ones atomically.
pub async fn replace_slots(
    pool: &PgPool,
    gdp_id: &str,
    slots: &[GdpSlotRow],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query("delete from tmu.gdp_slot where gdp_id = $1")
        .bind(gdp_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for s in slots {
        sqlx::query(
            "insert into tmu.gdp_slot \
               (gdp_id, callsign, dep, original_eta, cta, edct, delay_min) \
             values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(gdp_id)
        .bind(&s.callsign)
        .bind(&s.dep)
        .bind(s.original_eta)
        .bind(s.cta)
        .bind(s.edct)
        .bind(s.delay_min)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Insert or replace a single frozen slot (used to lock a pop-up's advisory EDCT).
pub async fn upsert_slot(pool: &PgPool, gdp_id: &str, s: &GdpSlotRow) -> Result<(), ApiError> {
    sqlx::query(
        "insert into tmu.gdp_slot \
           (gdp_id, callsign, dep, original_eta, cta, edct, delay_min) \
         values ($1, $2, $3, $4, $5, $6, $7) \
         on conflict (gdp_id, callsign) do update set \
           dep = excluded.dep, original_eta = excluded.original_eta, cta = excluded.cta, \
           edct = excluded.edct, delay_min = excluded.delay_min, assigned_at = now()",
    )
    .bind(gdp_id)
    .bind(&s.callsign)
    .bind(&s.dep)
    .bind(s.original_eta)
    .bind(s.cta)
    .bind(s.edct)
    .bind(s.delay_min)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Remove one frozen slot (unlock). Returns whether a row was removed.
pub async fn delete_slot(pool: &PgPool, gdp_id: &str, callsign: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from tmu.gdp_slot where gdp_id = $1 and callsign = $2")
        .bind(gdp_id)
        .bind(callsign)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}
