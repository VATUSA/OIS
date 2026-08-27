//! Flow Constrained Area (FCA) storage. Shared, server-side — one FCA set for everyone.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::models::{FcaBody, UpsertFcaRequest, UpsertRouteRequest};

const FCA_SELECT: &str = "select f.id, f.name, f.color, f.artcc, f.points, f.dests, \
    f.origins, f.fixes, f.scope, f.min_fl, f.max_fl, f.dir, f.mode, f.rate, f.mit, \
    f.enabled, f.manual_order, f.manual_seq, f.updated_at, u.display_name as updated_by \
    from flow.fca f left join identity.users u on u.id = f.updated_by";

pub async fn list_fcas(pool: &PgPool) -> Result<Vec<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!(
        "{FCA_SELECT} where f.deleted_at is null order by f.name"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// FCAs that existed and were enabled at instant `at` (for historical replay). Enable/disable isn't
/// historized, so the current `enabled` flag is used — an FCA toggled off since then is excluded.
pub async fn list_fcas_at(pool: &PgPool, at: DateTime<Utc>) -> Result<Vec<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!(
        "{FCA_SELECT} where f.enabled and f.created_at <= $1 \
           and (f.deleted_at is null or f.deleted_at > $1) order by f.name"
    ))
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_fca(pool: &PgPool, id: &str) -> Result<Option<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!("{FCA_SELECT} where f.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Bind every FCA column from a normalized request. Shared by insert + update.
fn bind_fca<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    req: &'q UpsertFcaRequest,
    actor: &'q str,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    q.bind(req.name.trim())
        .bind(req.color.as_deref().unwrap_or("#f59e0b"))
        .bind(req.artcc.trim().to_ascii_uppercase())
        .bind(sqlx::types::Json(&req.points))
        .bind(&req.dests)
        .bind(&req.origins)
        .bind(&req.fixes)
        .bind(&req.scope)
        .bind(req.min_fl)
        .bind(req.max_fl)
        .bind(req.dir.as_deref().unwrap_or("any"))
        .bind(req.mode.as_deref().unwrap_or("rate"))
        .bind(req.rate.unwrap_or(30).clamp(0, 240))
        .bind(req.mit.unwrap_or(15).clamp(0, 200))
        .bind(req.enabled.unwrap_or(true))
        .bind(actor)
}

pub async fn create_fca(
    pool: &PgPool,
    req: &UpsertFcaRequest,
    actor: &str,
) -> Result<String, ApiError> {
    let q = sqlx::query(
        "insert into flow.fca
             (name, color, artcc, points, dests, origins, fixes, scope, min_fl, max_fl,
              dir, mode, rate, mit, enabled, updated_by, created_by)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$16)
         returning id",
    );
    // bind_fca sets $1..$16 (the 16 shared columns, $16 = actor → updated_by);
    // created_by reuses $16 in the SQL, so no extra bind is needed.
    let row = bind_fca(q, req, actor)
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    use sqlx::Row;
    row.try_get::<String, _>("id")
        .map_err(|_| ApiError::Internal)
}

pub async fn update_fca(
    pool: &PgPool,
    id: &str,
    req: &UpsertFcaRequest,
    actor: &str,
) -> Result<bool, ApiError> {
    let q = sqlx::query(
        "update flow.fca set
             name = $1, color = $2, artcc = $3, points = $4, dests = $5, origins = $6,
             fixes = $7, scope = $8, min_fl = $9, max_fl = $10, dir = $11, mode = $12,
             rate = $13, mit = $14, enabled = $15, updated_by = $16
         where id = $17",
    );
    let result = bind_fca(q, req, actor)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_fca(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    // Soft-delete so the historical dashboard can still show the FCA during the window it existed.
    let result =
        sqlx::query("update flow.fca set deleted_at = now() where id = $1 and deleted_at is null")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- flow map routes (shared filed-route strings, resolved on read) ---

/// The stored fields of a route; the handler resolves `route` to a track for the API response.
#[derive(Debug, sqlx::FromRow)]
pub struct RouteRow {
    pub id: String,
    pub name: String,
    pub color: String,
    pub route: String,
    pub dep: String,
    pub arr: String,
    pub artcc: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub updated_by: Option<String>,
}

const ROUTE_SELECT: &str = "select r.id, r.name, r.color, r.route, r.dep, r.arr, r.artcc, \
    r.updated_at, u.display_name as updated_by \
    from flow.route r left join identity.users u on u.id = r.updated_by";

/// All routes, or — when `artcc` is given — that ARTCC's routes plus the global (NULL) ones.
pub async fn list_routes(pool: &PgPool, artcc: Option<&str>) -> Result<Vec<RouteRow>, ApiError> {
    let sql = match artcc {
        Some(_) => format!("{ROUTE_SELECT} where r.artcc = $1 or r.artcc is null order by r.name"),
        None => format!("{ROUTE_SELECT} order by r.name"),
    };
    let mut q = sqlx::query_as::<_, RouteRow>(&sql);
    if let Some(a) = artcc {
        q = q.bind(a);
    }
    q.fetch_all(pool).await.map_err(|_| ApiError::Internal)
}

pub async fn get_route(pool: &PgPool, id: &str) -> Result<Option<RouteRow>, ApiError> {
    sqlx::query_as::<_, RouteRow>(&format!("{ROUTE_SELECT} where r.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create_route(
    pool: &PgPool,
    req: &UpsertRouteRequest,
    actor: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into flow.route (name, color, route, dep, arr, artcc, updated_by, created_by) \
         values ($1, $2, $3, $4, $5, $6, $7, $7) returning id",
    )
    .bind(req.name.trim())
    .bind(req.color.as_deref().unwrap_or("#38bdf8"))
    .bind(req.route.trim())
    .bind(req.dep.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(req.arr.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(norm_artcc(req.artcc.as_deref()))
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn update_route(
    pool: &PgPool,
    id: &str,
    req: &UpsertRouteRequest,
    actor: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.route set name = $1, color = $2, route = $3, dep = $4, arr = $5, \
         artcc = $6, updated_by = $7 where id = $8",
    )
    .bind(req.name.trim())
    .bind(req.color.as_deref().unwrap_or("#38bdf8"))
    .bind(req.route.trim())
    .bind(req.dep.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(req.arr.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(norm_artcc(req.artcc.as_deref()))
    .bind(actor)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Normalize an ARTCC id: trimmed + uppercased, or `None` for blank (a global route).
pub fn norm_artcc(raw: Option<&str>) -> Option<String> {
    raw.map(|a| a.trim().to_ascii_uppercase())
        .filter(|a| !a.is_empty())
}

pub async fn delete_route(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from flow.route where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Set (or clear) an FCA's manual crossing order.
pub async fn set_manual_order(
    pool: &PgPool,
    id: &str,
    order: &[String],
    manual_seq: bool,
    actor: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.fca set manual_order = $2, manual_seq = $3, updated_by = $4 where id = $1",
    )
    .bind(id)
    .bind(order)
    .bind(manual_seq)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- frozen CFR releases ---

/// Frozen releases for an FCA as (callsign, cta_ms, edct_ms).
pub async fn list_releases(
    pool: &PgPool,
    fca_id: &str,
) -> Result<Vec<(String, i64, i64)>, ApiError> {
    sqlx::query_as::<_, (String, i64, i64)>(
        "select callsign, cta_ms, edct_ms from flow.fca_release where fca_id = $1",
    )
    .bind(fca_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Earliest frozen FCA release (EDCT, epoch-ms) per callsign for the given callsigns, across
/// *enabled* FCAs only. Lets the departure-field view surface FCA-issued release times, so an FCA's
/// RDY/RLSD flows to the airport departures list — not just the FCA page.
pub async fn releases_for_callsigns(
    pool: &PgPool,
    callsigns: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    if callsigns.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (String, i64)>(
        "select r.callsign, min(r.edct_ms) as edct \
         from flow.fca_release r join flow.fca f on f.id = r.fca_id \
         where f.enabled and r.callsign = any($1) \
         group by r.callsign",
    )
    .bind(callsigns)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().collect())
}

pub async fn upsert_release(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
    cta_ms: i64,
    edct_ms: i64,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms, updated_by)
         values ($1, $2, $3, $4, $5)
         on conflict (fca_id, callsign) do update set
             cta_ms = excluded.cta_ms, edct_ms = excluded.edct_ms, updated_by = excluded.updated_by",
    )
    .bind(fca_id)
    .bind(callsign)
    .bind(cta_ms)
    .bind(edct_ms)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn delete_release(pool: &PgPool, fca_id: &str, callsign: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from flow.fca_release where fca_id = $1 and callsign = $2")
        .bind(fca_id)
        .bind(callsign)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}
