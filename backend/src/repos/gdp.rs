//! Ground Delay Program persistence — the program (lifecycle like ground stops) plus its
//! frozen control-time slots (written at publish so issued EDCTs don't drift).

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{errors::ApiError, models::GdpBody};

const GDP_SELECT: &str = "select g.id, g.airport, g.aar, g.start_time, g.end_time, \
    g.max_enroute_min, g.exempt_airborne, g.status, g.published_at, g.updated_at, \
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
    start_time: &str,
    end_time: &str,
    max_enroute_min: Option<i32>,
    exempt_airborne: bool,
    actor: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into tmu.gdp \
           (airport, aar, start_time, end_time, max_enroute_min, exempt_airborne, \
            created_by, updated_by) \
         values ($1, $2, $3, $4, $5, $6, $7, $7) returning id",
    )
    .bind(airport)
    .bind(aar)
    .bind(start_time)
    .bind(end_time)
    .bind(max_enroute_min)
    .bind(exempt_airborne)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
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
        "update tmu.gdp set status = 'cancelled' \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_gdp(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from tmu.gdp where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
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
