//! Flow Constrained Area (FCA) storage. Shared, server-side — one FCA set for everyone.

use crate::auth::principal::Attribution;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::models::{FcaBody, UpsertFcaRequest, UpsertRouteRequest};

const FCA_SELECT: &str = "select f.id, f.name, f.color, f.artcc, f.points, f.dests, \
    f.origins, f.fixes, f.scope, f.min_fl, f.max_fl, f.dir, f.mode, f.rate, f.mit, \
    f.enabled, f.manual_order, f.manual_seq, f.updated_at, \
    coalesce(u.display_name, a.display_name) as updated_by, \
    f.event_id, f.event_status, f.auto_publish \
    from flow.fca f left join identity.users u on u.id = f.updated_by \
    left join access.actors a on a.id = f.updated_by_actor";

/// Event FCAs are hidden from every live map and the metering engine until they're `published`;
/// planned + archived ones are only ever seen in their event's builder ([`list_event_fcas`]).
const NOT_HIDDEN_EVENT: &str = "(f.event_id is null or f.event_status = 'published')";

pub async fn list_fcas(pool: &PgPool) -> Result<Vec<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!(
        "{FCA_SELECT} where f.deleted_at is null and {NOT_HIDDEN_EVENT} order by f.name"
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
           and (f.deleted_at is null or f.deleted_at > $1) and {NOT_HIDDEN_EVENT} order by f.name"
    ))
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Every FCA belonging to one event — planned, published, and archived — for the event manager's
/// builder. Unlike [`list_fcas`] this ignores the publish gate (that's the whole point of the builder).
pub async fn list_event_fcas(pool: &PgPool, event_id: i64) -> Result<Vec<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!(
        "{FCA_SELECT} where f.event_id = $1 and f.deleted_at is null \
           order by f.event_status, f.name"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Whether the FCA `id` is soft-deleted. `false` for an id that doesn't exist, which the caller has
/// already turned into a 404 through [`get_fca`].
pub async fn fca_is_deleted(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>("select deleted_at is not null from flow.fca where id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(|deleted| deleted.unwrap_or(false))
        .map_err(|_| ApiError::Internal)
}

pub async fn get_fca(pool: &PgPool, id: &str) -> Result<Option<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!("{FCA_SELECT} where f.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// An FCA's colour when none is given: `--series-3` (Amber) in the dark theme, the canonical one. It was
/// `#f59e0b`, which was no token and none of the offered swatches (#698).
pub const DEFAULT_FCA_COLOR: &str = "#efc14d";

/// The lowest contrast an FCA colour may have against the dark ground `#08080a` (WCAG 2.x ratio). Every
/// `--series-*` and `--ink-3` value clears it in both themes (the lowest is 3.8); black and near-black
/// don't, so an FCA can't be saved invisible on the dark map (#698).
pub const MIN_GROUND_CONTRAST: f64 = 3.0;

/// An FCA colour as stored: trimmed, lowercase, or the default when none is given.
pub fn normalize_fca_color(raw: Option<&str>) -> String {
    match raw.map(str::trim) {
        Some(c) if !c.is_empty() => c.to_ascii_lowercase(),
        _ => DEFAULT_FCA_COLOR.to_string(),
    }
}

/// [`normalize_fca_color`], refused unless it is `#rrggbb` and clears [`MIN_GROUND_CONTRAST`]. The map
/// parses exactly that shape, so anything else would draw grey on the map while its list chip showed the
/// raw value (#698).
pub fn fca_color(raw: Option<&str>) -> Result<String, ApiError> {
    let c = normalize_fca_color(raw);
    let rgb = srgb(&c).ok_or(ApiError::BadRequest)?;
    let ground = srgb(DARK_GROUND).expect("a valid constant");
    if contrast(luminance(rgb), luminance(ground)) < MIN_GROUND_CONTRAST {
        return Err(ApiError::BadRequest);
    }
    Ok(c)
}

/// The dark theme's `--ground`, the background an FCA must stay visible on.
const DARK_GROUND: &str = "#08080a";

/// `#rrggbb` as sRGB channels in `0..=1`; `None` for any other shape.
fn srgb(hex: &str) -> Option<[f64; 3]> {
    let h = hex
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let channel = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map(|v| f64::from(v) / 255.0);
    Some([channel(0).ok()?, channel(2).ok()?, channel(4).ok()?])
}

/// WCAG relative luminance.
fn luminance(rgb: [f64; 3]) -> f64 {
    let lin = |c: f64| {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/// WCAG contrast ratio between two luminances.
fn contrast(a: f64, b: f64) -> f64 {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// Bind every FCA column from a normalized request. Shared by insert + update.
fn bind_fca<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    req: &'q UpsertFcaRequest,
    by: &'q Attribution,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    q.bind(req.name.trim())
        .bind(normalize_fca_color(req.color.as_deref()))
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
        .bind(&by.user_id)
        .bind(&by.actor_id)
}

pub async fn create_fca(
    pool: &PgPool,
    req: &UpsertFcaRequest,
    by: &Attribution,
) -> Result<String, ApiError> {
    let q = sqlx::query(
        "insert into flow.fca
             (name, color, artcc, points, dests, origins, fixes, scope, min_fl, max_fl,
              dir, mode, rate, mit, enabled, updated_by, created_by,
              updated_by_actor, created_by_actor)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$16,$17,$17)
         returning id",
    );
    // bind_fca sets $1..$17 (the 15 shared columns, then the user → `*_by` and the actor →
    // `*_by_actor`); the created_* columns reuse $16/$17 in the SQL, so no extra bind is needed.
    let row = bind_fca(q, req, by)
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    use sqlx::Row;
    row.try_get::<String, _>("id")
        .map_err(|_| ApiError::Internal)
}

/// Create an FCA owned by an event. Same columns as [`create_fca`], plus `event_id` and a starting
/// `event_status = 'planned'` — so it's invisible on live maps until published.
pub async fn create_event_fca(
    pool: &PgPool,
    event_id: i64,
    req: &UpsertFcaRequest,
    by: &Attribution,
) -> Result<String, ApiError> {
    let q = sqlx::query(
        "insert into flow.fca
             (name, color, artcc, points, dests, origins, fixes, scope, min_fl, max_fl,
              dir, mode, rate, mit, enabled, updated_by, created_by,
              updated_by_actor, created_by_actor, event_id, event_status)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$16,$17,$17,$18,'planned')
         returning id",
    );
    let row = bind_fca(q, req, by)
        .bind(event_id)
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
    by: &Attribution,
) -> Result<bool, ApiError> {
    let q = sqlx::query(
        "update flow.fca set
             name = $1, color = $2, artcc = $3, points = $4, dests = $5, origins = $6,
             fixes = $7, scope = $8, min_fl = $9, max_fl = $10, dir = $11, mode = $12,
             rate = $13, mit = $14, enabled = $15, updated_by = $16, updated_by_actor = $17
         where id = $18",
    );
    let result = bind_fca(q, req, by)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Publish an event FCA (planned → published) so it shows on live maps; stamps `published_at`.
/// Scoped to the event; false if it isn't there or isn't currently `planned`.
pub async fn mark_event_fca_published(
    pool: &PgPool,
    event_id: i64,
    fca_id: &str,
) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "update flow.fca set event_status = 'published', published_at = now() \
         where id = $1 and event_id = $2 and event_status = 'planned'",
    )
    .bind(fca_id)
    .bind(event_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// Archive an event FCA (planned/published → archived); stamps `archived_at`. Kept as event history,
/// hidden from every live map. Scoped to the event; false if it isn't there or is already archived.
pub async fn mark_event_fca_archived(
    pool: &PgPool,
    event_id: i64,
    fca_id: &str,
) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "update flow.fca set event_status = 'archived', archived_at = now() \
         where id = $1 and event_id = $2 and event_status in ('planned', 'published')",
    )
    .bind(fca_id)
    .bind(event_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// Set an event FCA's auto-publish flag. Scoped to the event.
pub async fn set_event_fca_auto(
    pool: &PgPool,
    event_id: i64,
    fca_id: &str,
    auto: bool,
) -> Result<bool, ApiError> {
    let r = sqlx::query("update flow.fca set auto_publish = $3 where id = $1 and event_id = $2")
        .bind(fca_id)
        .bind(event_id)
        .bind(auto)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// One automatic-lifecycle pass (run periodically by the scheduler):
///   * publish planned + `auto_publish` FCAs once their event is within 30 min of starting (and hasn't
///     ended yet — so a missed window still recovers on the next tick);
///   * archive any still-live (planned/published) FCA whose event has ended.
///
/// Returns the number of rows changed, so the caller can skip the realtime nudge when nothing moved.
pub async fn run_event_fca_lifecycle(pool: &PgPool) -> Result<u64, ApiError> {
    let published = sqlx::query(
        "update flow.fca f set event_status = 'published', published_at = now() \
         from events.event e \
         where f.event_id = e.id and f.event_status = 'planned' and f.auto_publish \
           and now() >= e.start_time - interval '30 minutes' and now() < e.end_time \
           and f.deleted_at is null",
    )
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let archived = sqlx::query(
        "update flow.fca f set event_status = 'archived', archived_at = now() \
         from events.event e \
         where f.event_id = e.id and f.event_status in ('planned', 'published') \
           and now() >= e.end_time and f.deleted_at is null",
    )
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(published.rows_affected() + archived.rows_affected())
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
    r.updated_at, coalesce(u.display_name, a.display_name) as updated_by \
    from flow.route r left join identity.users u on u.id = r.updated_by \
    left join access.actors a on a.id = r.updated_by_actor";

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
    by: &Attribution,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into flow.route (name, color, route, dep, arr, artcc, updated_by, created_by, \
             updated_by_actor, created_by_actor) \
         values ($1, $2, $3, $4, $5, $6, $7, $7, $8, $8) returning id",
    )
    .bind(req.name.trim())
    .bind(req.color.as_deref().unwrap_or("#38bdf8"))
    .bind(req.route.trim())
    .bind(req.dep.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(req.arr.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(norm_artcc(req.artcc.as_deref()))
    .bind(&by.user_id)
    .bind(&by.actor_id)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn update_route(
    pool: &PgPool,
    id: &str,
    req: &UpsertRouteRequest,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.route set name = $1, color = $2, route = $3, dep = $4, arr = $5, \
         artcc = $6, updated_by = $7, updated_by_actor = $8 where id = $9",
    )
    .bind(req.name.trim())
    .bind(req.color.as_deref().unwrap_or("#38bdf8"))
    .bind(req.route.trim())
    .bind(req.dep.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(req.arr.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(norm_artcc(req.artcc.as_deref()))
    .bind(&by.user_id)
    .bind(&by.actor_id)
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
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.fca set manual_order = $2, manual_seq = $3, updated_by = $4, \
             updated_by_actor = $5 where id = $1",
    )
    .bind(id)
    .bind(order)
    .bind(manual_seq)
    .bind(&by.user_id)
    .bind(&by.actor_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- frozen CFR releases ---

/// A writer's precondition on a release (#585): `If-None-Match: *` is [`Expect::Absent`], and
/// `If-Match: N` is [`Expect::Version`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Absent,
    Version(i64),
}

/// Who holds a release now, and at what version (#585). `machine` is the holding actor's id and name
/// when a service account or API key wrote it last; `None` means a person did, including a legacy row
/// that predates actor attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseHolder {
    pub version: i64,
    pub machine: Option<(String, String)>,
}

const HOLDER_SELECT: &str = "select r.callsign, r.version, \
        case when a.actor_type in ('service_account', 'api_key') then a.id end, \
        case when a.actor_type in ('service_account', 'api_key') then a.display_name end \
     from flow.fca_release r left join access.actors a on a.id = r.updated_by_actor";

fn holder_of(version: i64, id: Option<String>, name: Option<String>) -> ReleaseHolder {
    ReleaseHolder {
        version,
        machine: id.map(|id| (id, name.unwrap_or_default())),
    }
}

/// The holder of one release, or `None` when the flight is not released.
pub async fn release_holder(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
) -> Result<Option<ReleaseHolder>, ApiError> {
    let row = sqlx::query_as::<_, (String, i64, Option<String>, Option<String>)>(&format!(
        "{HOLDER_SELECT} where r.fca_id = $1 and r.callsign = $2"
    ))
    .bind(fca_id)
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(row.map(|(_, v, id, name)| holder_of(v, id, name)))
}

/// Every release's holder in one FCA, by callsign — read alongside the metering input, never part of
/// it, so provenance and versions cannot change a computed time.
pub async fn release_holders(
    pool: &PgPool,
    fca_id: &str,
) -> Result<HashMap<String, ReleaseHolder>, ApiError> {
    let rows = sqlx::query_as::<_, (String, i64, Option<String>, Option<String>)>(&format!(
        "{HOLDER_SELECT} where r.fca_id = $1"
    ))
    .bind(fca_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(cs, v, id, name)| (cs, holder_of(v, id, name)))
        .collect())
}

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

/// The locked wheels-up (epoch ms) of each of `callsigns` that holds one, for the sector occupancy
/// engine's proposed population (#721): the **latest** of its issued CFR, its releases in live FCAs (enabled,
/// not deleted) and its slot in a published GDP. The latest is the binding constraint — a flight held for
/// a later release can't satisfy an earlier one — and it is the flight advisory's rule too
/// (`handlers::flow::flight_advisory`, `edcts.max()`). The departures list uses it too (#732).
pub async fn locked_wheels_up(
    pool: &PgPool,
    callsigns: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    if callsigns.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (String, i64)>(
        "select callsign, max(t)::bigint from ( \
             select callsign, (extract(epoch from wheels_up) * 1000)::bigint as t \
             from tmu.issued_cfrs where callsign = any($1) \
             union all \
             select r.callsign, r.edct_ms from flow.fca_release r join flow.fca f on f.id = r.fca_id \
             where f.enabled and f.deleted_at is null and r.callsign = any($1) \
             union all \
             select s.callsign, (extract(epoch from s.edct) * 1000)::bigint \
             from tmu.gdp_slot s join tmu.gdp g on g.id = s.gdp_id \
             where g.status = 'published' and s.edct is not null and s.callsign = any($1) \
         ) locked group by callsign",
    )
    .bind(callsigns)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().collect())
}

/// Write a release, returning its new version — or `None` when `expect` did not hold, in which case
/// nothing was written (#585). The precondition is checked **in the write itself**, so there is no
/// window between reading the version and changing the row.
pub async fn upsert_release(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
    cta_ms: i64,
    edct_ms: i64,
    by: &Attribution,
    expect: Option<Expect>,
) -> Result<Option<i64>, ApiError> {
    let sql = match expect {
        // Unconditional (a person, as before): create or replace.
        None => {
            "insert into flow.fca_release as r (fca_id, callsign, cta_ms, edct_ms, updated_by, updated_by_actor)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (fca_id, callsign) do update set
                 cta_ms = excluded.cta_ms, edct_ms = excluded.edct_ms,
                 updated_by = excluded.updated_by, updated_by_actor = excluded.updated_by_actor,
                 version = r.version + 1
             returning r.version"
        }
        // Create only: an existing release makes this a no-op, so a retry cannot issue twice.
        Some(Expect::Absent) => {
            "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms, updated_by, updated_by_actor)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (fca_id, callsign) do nothing
             returning version"
        }
        // Replace only the version the writer last saw.
        Some(Expect::Version(_)) => {
            "update flow.fca_release set
                 cta_ms = $3, edct_ms = $4, updated_by = $5, updated_by_actor = $6,
                 version = version + 1
             where fca_id = $1 and callsign = $2 and version = $7
               and ($8::text is null or updated_by_actor = $8)
             returning version"
        }
    };
    let query = sqlx::query_scalar::<_, i64>(sql)
        .bind(fca_id)
        .bind(callsign)
        .bind(cta_ms)
        .bind(edct_ms)
        .bind(&by.user_id)
        .bind(&by.actor_id);
    // `$7` exists only in the conditional update; binding it elsewhere is a parameter-count error.
    // `$7`/`$8` exist only in the conditional update; binding them elsewhere is a parameter-count
    // error. `$8` is the machine that must already hold the row (#585 review).
    let query = match expect {
        Some(Expect::Version(v)) => query.bind(v).bind(by.machine_actor()),
        _ => query,
    };
    query
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Exchange two releases' frozen times within one FCA (#514).
///
/// Returns whether the swap happened — `false` means at least one of the two callsigns holds no
/// release, and nothing was written.
///
/// # One statement, deliberately
///
/// `update … from` over the same table is atomic by itself and reads the **pre-statement snapshot**,
/// so each row receives the other's values. A read-then-write pair would need an explicit
/// transaction and could still interleave with a concurrent [`upsert_release`], leaving one row
/// holding the other's time and the other holding its own.
///
/// `rows_affected() == 2` is the success condition rather than a separate existence check: if either
/// callsign has no release the self-join matches nothing, zero rows change, and a partial swap is
/// impossible. Passing the same callsign twice matches one row, which is also not 2 — though the
/// handler rejects that earlier with a clearer error.
///
/// # Why this is not the reorder
///
/// `set_manual_order` is the other way to change who goes first, and it is defined to re-chain
/// everyone behind the moved aircraft (`feed::fca`'s manual branch). This touches two rows and
/// nothing else, which is what lets two flights trade slots without renumbering the field.
pub async fn swap_releases(
    pool: &PgPool,
    fca_id: &str,
    a: &str,
    b: &str,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.fca_release r \
            set cta_ms = o.cta_ms, edct_ms = o.edct_ms, updated_by = $4, updated_by_actor = $5, \
                version = r.version + 1 \
           from flow.fca_release o \
          where r.fca_id = $1 and o.fca_id = $1 \
            and ((r.callsign = $2 and o.callsign = $3) \
              or (r.callsign = $3 and o.callsign = $2)) \
            and ($6::text is null or (r.updated_by_actor = $6 and o.updated_by_actor = $6))",
    )
    .bind(fca_id)
    .bind(a)
    .bind(b)
    .bind(&by.user_id)
    .bind(&by.actor_id)
    // A machine may swap only two releases it holds itself, decided in the write (#585 review).
    .bind(by.machine_actor())
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() == 2)
}

/// Clear a release, only at `version` when given and, for a machine, only if it already holds it
/// (#585). The holder comes from `by` here, as in the update and swap writers, rather than from the
/// caller: no route can reach the race this clause closes, so a call site that dropped it would go
/// unnoticed. Returns whether a row was removed.
pub async fn delete_release(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
    version: Option<i64>,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "delete from flow.fca_release \
         where fca_id = $1 and callsign = $2 and ($3::bigint is null or version = $3) \
           and ($4::text is null or updated_by_actor = $4)",
    )
    .bind(fca_id)
    .bind(callsign)
    .bind(version)
    .bind(by.machine_actor())
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod fca_color_tests {
    use super::{DEFAULT_FCA_COLOR, fca_color};

    #[test]
    fn a_colour_is_stored_trimmed_and_lowercase_or_defaulted() {
        assert_eq!(fca_color(Some(" #EFC14D ")).unwrap(), "#efc14d");
        assert_eq!(fca_color(None).unwrap(), DEFAULT_FCA_COLOR);
        assert_eq!(fca_color(Some("  ")).unwrap(), DEFAULT_FCA_COLOR);
    }

    /// Only the shape the map parses: `#abc` and `red` render as list chips but grey on the map.
    #[test]
    fn only_six_digit_hex_is_accepted() {
        for bad in [
            "red",
            "#abc",
            "efc14d",
            "#efc14d00",
            "#gggggg",
            "rgb(1,2,3)",
        ] {
            assert!(fca_color(Some(bad)).is_err(), "{bad}");
        }
    }

    /// The contrast floor against the dark ground: black and the ground itself are refused; every token
    /// swatch, in both themes, is not (the lowest is dark `--ink-3` at 3.8:1).
    #[test]
    fn an_invisible_colour_is_refused_and_every_token_passes() {
        for dark in ["#000000", "#08080a", "#333333", "#454545"] {
            assert!(fca_color(Some(dark)).is_err(), "{dark}");
        }
        for token in [
            "#1b8fb0", "#1f9d63", "#b7791f", "#8e5bd0", "#d0556b", "#3565d6", "#c2621a", "#5f8f2a",
            "#9898a2", "#5ec8e5", "#43d089", "#efc14d", "#c792ea", "#f07178", "#7b9dff", "#f5a83d",
            "#a3d977", "#6b6b74",
        ] {
            assert!(fca_color(Some(token)).is_ok(), "{token}");
        }
    }
}

#[cfg(test)]
mod locked_wheels_up_tests {
    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::PgPool;

    use super::locked_wheels_up;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 5, h, m, 0).unwrap()
    }

    async fn fca(pool: &PgPool, enabled: bool, deleted: bool) -> String {
        sqlx::query_scalar(
            "insert into flow.fca (enabled, deleted_at) values ($1, case when $2 then now() end) \
             returning id",
        )
        .bind(enabled)
        .bind(deleted)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn release(pool: &PgPool, fca_id: &str, callsign: &str, edct: DateTime<Utc>) {
        sqlx::query(
            "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms) values ($1, $2, $3, $3)",
        )
        .bind(fca_id)
        .bind(callsign)
        .bind(edct.timestamp_millis())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn cfr(pool: &PgPool, callsign: &str, wheels_up: DateTime<Utc>) {
        sqlx::query(
            "insert into tmu.issued_cfrs (callsign, airport, wheels_up) values ($1, 'KJFK', $2)",
        )
        .bind(callsign)
        .bind(wheels_up)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn gdp_slot(pool: &PgPool, status: &str, callsign: &str, edct: DateTime<Utc>) {
        let gdp: String = sqlx::query_scalar(
            "insert into tmu.gdp (airport, aar, start_time, end_time, status) \
             values ('KJFK', 30, '1400', '1800', $1) returning id",
        )
        .bind(status)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into tmu.gdp_slot (gdp_id, callsign, original_eta, cta, edct) \
             values ($1, $2, $3, $3, $3)",
        )
        .bind(&gdp)
        .bind(callsign)
        .bind(edct)
        .execute(pool)
        .await
        .unwrap();
    }

    /// #721 AC7: a flight holding conflicting locked times integrates from the **latest** — across two
    /// FCAs' releases, and across a CFR, a release and a GDP slot. Disabled or deleted FCAs and unpublished
    /// GDPs hold nothing, and a callsign with nothing locked (or not asked for) is absent.
    #[sqlx::test]
    async fn the_latest_locked_time_wins_across_every_source(pool: PgPool) {
        let (a, b) = (fca(&pool, true, false).await, fca(&pool, true, false).await);
        let disabled = fca(&pool, false, false).await;
        let deleted = fca(&pool, true, true).await;

        release(&pool, &a, "TWO", at(15, 0)).await; // two FCAs disagree
        release(&pool, &b, "TWO", at(14, 30)).await;

        cfr(&pool, "MIX", at(14, 40)).await; // CFR < release < GDP
        release(&pool, &a, "MIX", at(14, 45)).await;
        gdp_slot(&pool, "published", "MIX", at(15, 15)).await;

        cfr(&pool, "CFR", at(15, 30)).await; // the CFR is the latest
        release(&pool, &a, "CFR", at(15, 0)).await;

        release(&pool, &a, "LIVE", at(14, 50)).await; // later times that don't count
        release(&pool, &disabled, "LIVE", at(16, 0)).await;
        release(&pool, &deleted, "LIVE", at(16, 30)).await;
        gdp_slot(&pool, "draft", "LIVE", at(17, 0)).await;

        release(&pool, &a, "UNASKED", at(15, 0)).await;

        let asked: Vec<String> = ["TWO", "MIX", "CFR", "LIVE", "NONE"]
            .map(String::from)
            .into();
        let locked = locked_wheels_up(&pool, &asked).await.unwrap();

        let ms = |t: DateTime<Utc>| Some(t.timestamp_millis());
        assert_eq!(
            locked.get("TWO").copied(),
            ms(at(15, 0)),
            "the later of two FCAs' releases"
        );
        assert_eq!(
            locked.get("MIX").copied(),
            ms(at(15, 15)),
            "the GDP slot, latest of three"
        );
        assert_eq!(
            locked.get("CFR").copied(),
            ms(at(15, 30)),
            "the CFR, latest of two"
        );
        assert_eq!(
            locked.get("LIVE").copied(),
            ms(at(14, 50)),
            "only the live FCA's release counts"
        );
        assert!(!locked.contains_key("NONE"), "nothing locked");
        assert!(!locked.contains_key("UNASKED"), "not asked for");
        assert!(locked_wheels_up(&pool, &[]).await.unwrap().is_empty());
    }
}
