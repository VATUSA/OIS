//! Event cache repo. The `events.event` table mirrors the upcoming VATUSA events the
//! sync job pulls in; per-event planning tables (added in later passes) reference it.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use serde_json::Value;

use crate::errors::ApiError;
use crate::models::{
    AirportRateBody, DccRequestBody, EventBody, FacilitySupportBody, TmiPackageBody,
    TmiPackageItemBody,
};

const EVENT_SELECT: &str = "select id, title, body, banner_image_url, facility, \
    start_time, end_time, review_status from events.event";

/// All cached events (upcoming, in-progress, and recently-ended within the sync's retention window),
/// soonest first. The client splits these into upcoming/past; the cache is already bounded by the
/// sync's prune, so this stays small. Carries the at-a-glance status flags (`recording`,
/// `ace_requested`, `facility_support`) the planning list shows as badges.
pub async fn list_all(pool: &PgPool) -> Result<Vec<EventBody>, ApiError> {
    sqlx::query_as::<_, EventBody>(
        "select e.id, e.title, e.body, e.banner_image_url, e.facility, e.start_time, e.end_time, \
                e.review_status, \
                case \
                    when lc.status = 'open'  then 'recording' \
                    when lc.status = 'saved' then 'recorded' \
                    when ec.enabled          then 'scheduled' \
                    else 'off' \
                end as recording, \
                exists (select 1 from ace.requests ar \
                        where ar.event_id = e.id and ar.status <> 'cancelled') \
                    as ace_requested, \
                exists (select 1 from events.facility_support fs where fs.event_id = e.id) \
                    as facility_support \
         from events.event e \
         left join stats.event_capture ec on ec.event_id = e.id \
         left join lateral ( \
             select status from stats.capture \
             where event_id = e.id and status <> 'discarded' \
             order by start_time desc limit 1 \
         ) lc on true \
         order by e.start_time",
    )
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

// --- facility support matrix ---

const FS_SELECT: &str = "select f.facility, f.level, f.notes, f.updated_at, \
    u.display_name as updated_by \
    from events.facility_support f left join identity.users u on u.id = f.updated_by";

pub async fn list_facility_support(
    pool: &PgPool,
    event_id: i64,
) -> Result<Vec<FacilitySupportBody>, ApiError> {
    sqlx::query_as::<_, FacilitySupportBody>(&format!(
        "{FS_SELECT} where f.event_id = $1 order by f.facility"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_facility_support(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
) -> Result<Option<FacilitySupportBody>, ApiError> {
    sqlx::query_as::<_, FacilitySupportBody>(&format!(
        "{FS_SELECT} where f.event_id = $1 and f.facility = $2"
    ))
    .bind(event_id)
    .bind(facility)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn upsert_facility_support(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
    level: &str,
    notes: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.facility_support (event_id, facility, level, notes, updated_by)
         values ($1, $2, $3, $4, $5)
         on conflict (event_id, facility) do update set
             level = excluded.level,
             notes = excluded.notes,
             updated_by = excluded.updated_by",
    )
    .bind(event_id)
    .bind(facility)
    .bind(level)
    .bind(notes)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn delete_facility_support(
    pool: &PgPool,
    event_id: i64,
    facility: &str,
) -> Result<bool, ApiError> {
    let result =
        sqlx::query("delete from events.facility_support where event_id = $1 and facility = $2")
            .bind(event_id)
            .bind(facility)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- airport rates (AAR/ADR) ---

const RATE_SELECT: &str = "select r.icao, r.aar, r.adr, r.artcc, r.config_id, r.source, \
    r.updated_at, u.display_name as updated_by \
    from events.airport_rate r left join identity.users u on u.id = r.updated_by";

pub async fn list_airport_rates(
    pool: &PgPool,
    event_id: i64,
) -> Result<Vec<AirportRateBody>, ApiError> {
    sqlx::query_as::<_, AirportRateBody>(&format!(
        "{RATE_SELECT} where r.event_id = $1 order by r.icao"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_airport_rate(
    pool: &PgPool,
    event_id: i64,
    icao: &str,
) -> Result<Option<AirportRateBody>, ApiError> {
    sqlx::query_as::<_, AirportRateBody>(&format!(
        "{RATE_SELECT} where r.event_id = $1 and r.icao = $2"
    ))
    .bind(event_id)
    .bind(icao)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_airport_rate(
    pool: &PgPool,
    event_id: i64,
    icao: &str,
    aar: i32,
    adr: i32,
    artcc: &str,
    config_id: Option<&str>,
    source: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.airport_rate (event_id, icao, aar, adr, artcc, config_id, source, updated_by)
         values ($1, $2, $3, $4, $5, $6, $7, $8)
         on conflict (event_id, icao) do update set
             aar = excluded.aar,
             adr = excluded.adr,
             artcc = excluded.artcc,
             config_id = excluded.config_id,
             source = excluded.source,
             updated_by = excluded.updated_by",
    )
    .bind(event_id)
    .bind(icao)
    .bind(aar)
    .bind(adr)
    .bind(artcc)
    .bind(config_id)
    .bind(source)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn delete_airport_rate(
    pool: &PgPool,
    event_id: i64,
    icao: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from events.airport_rate where event_id = $1 and icao = $2")
        .bind(event_id)
        .bind(icao)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- TMI packages ---

/// A package's own columns (items are loaded separately).
#[derive(sqlx::FromRow)]
struct PackageRow {
    id: String,
    name: String,
    status: String,
    auto_publish: bool,
    activated_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    updated_by: Option<String>,
}

const PACKAGE_SELECT: &str = "select p.id, p.name, p.status, p.auto_publish, p.activated_at, \
    p.archived_at, p.updated_at, u.display_name as updated_by \
    from events.tmi_package p left join identity.users u on u.id = p.updated_by";

/// A package item plus the live-row reference recorded at activation (for deactivation cleanup).
#[derive(sqlx::FromRow)]
pub struct PackageItemRef {
    pub kind: String,
    #[sqlx(rename = "payload")]
    pub payload: sqlx::types::Json<Value>,
    pub live_ref: Option<String>,
}

pub async fn list_package_items(
    pool: &PgPool,
    package_id: &str,
) -> Result<Vec<TmiPackageItemBody>, ApiError> {
    sqlx::query_as::<_, TmiPackageItemBody>(
        "select id, kind, payload from events.tmi_package_item \
         where package_id = $1 order by created_at",
    )
    .bind(package_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn list_packages(pool: &PgPool, event_id: i64) -> Result<Vec<TmiPackageBody>, ApiError> {
    let rows = sqlx::query_as::<_, PackageRow>(&format!(
        "{PACKAGE_SELECT} where p.event_id = $1 order by p.created_at"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    let mut packages = Vec::with_capacity(rows.len());
    for r in rows {
        let items = list_package_items(pool, &r.id).await?;
        packages.push(TmiPackageBody {
            id: r.id,
            name: r.name,
            status: r.status,
            auto_publish: r.auto_publish,
            activated_at: r.activated_at,
            archived_at: r.archived_at,
            updated_at: r.updated_at,
            updated_by: r.updated_by,
            items,
        });
    }
    Ok(packages)
}

/// One package (for scope/ownership checks). Returns (event_id, status).
pub async fn get_package_owner(
    pool: &PgPool,
    package_id: &str,
) -> Result<Option<(i64, String)>, ApiError> {
    sqlx::query_as::<_, (i64, String)>(
        "select event_id, status from events.tmi_package where id = $1",
    )
    .bind(package_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn create_package(
    pool: &PgPool,
    event_id: i64,
    name: &str,
    actor: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into events.tmi_package (event_id, name, updated_by) \
         values ($1, $2, $3) returning id",
    )
    .bind(event_id)
    .bind(name)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn delete_package(pool: &PgPool, package_id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from events.tmi_package where id = $1")
        .bind(package_id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn add_package_item(
    pool: &PgPool,
    package_id: &str,
    kind: &str,
    payload: &Value,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into events.tmi_package_item (package_id, kind, payload) \
         values ($1, $2, $3) returning id",
    )
    .bind(package_id)
    .bind(kind)
    .bind(sqlx::types::Json(payload))
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn delete_package_item(
    pool: &PgPool,
    package_id: &str,
    item_id: &str,
) -> Result<bool, ApiError> {
    let result =
        sqlx::query("delete from events.tmi_package_item where package_id = $1 and id = $2")
            .bind(package_id)
            .bind(item_id)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn mark_package_activated(
    pool: &PgPool,
    package_id: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "update events.tmi_package set status = 'activated', activated_at = now(), \
         updated_by = $2 where id = $1",
    )
    .bind(package_id)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Record the live row an item materialized to (its tmu id, or ICAO for programs) so a later
/// deactivation can cancel exactly what was created.
pub async fn set_item_live_ref(
    pool: &PgPool,
    item_id: &str,
    live_ref: &str,
) -> Result<(), ApiError> {
    sqlx::query("update events.tmi_package_item set live_ref = $2 where id = $1")
        .bind(item_id)
        .bind(live_ref)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// The items of a package with their live-row refs (for deactivation cleanup).
pub async fn list_package_item_refs(
    pool: &PgPool,
    package_id: &str,
) -> Result<Vec<PackageItemRef>, ApiError> {
    sqlx::query_as::<_, PackageItemRef>(
        "select kind, payload, live_ref from events.tmi_package_item \
         where package_id = $1 order by created_at",
    )
    .bind(package_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Archive a package after its live rows were cancelled; clears the now-stale live refs.
pub async fn mark_package_archived(
    pool: &PgPool,
    package_id: &str,
    actor: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query(
        "update events.tmi_package set status = 'archived', archived_at = now(), \
         updated_by = $2 where id = $1",
    )
    .bind(package_id)
    .bind(actor)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    sqlx::query("update events.tmi_package_item set live_ref = null where package_id = $1")
        .bind(package_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Toggle a package's auto-publish flag.
pub async fn set_package_auto(
    pool: &PgPool,
    package_id: &str,
    auto: bool,
) -> Result<bool, ApiError> {
    let r = sqlx::query("update events.tmi_package set auto_publish = $2 where id = $1")
        .bind(package_id)
        .bind(auto)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// `(package_id, event_id, actor)` for a package the scheduler should act on. `actor` is the package's
/// `updated_by` (the human who last touched it); rows with no attributable actor are skipped.
pub type SchedulablePackage = (String, i64, String);

/// Draft + auto packages whose event is within 30 min of starting (and hasn't ended) — auto-activate.
pub async fn auto_due_packages(pool: &PgPool) -> Result<Vec<SchedulablePackage>, ApiError> {
    sqlx::query_as::<_, SchedulablePackage>(
        "select p.id, p.event_id, p.updated_by from events.tmi_package p \
         join events.event e on e.id = p.event_id \
         where p.status = 'draft' and p.auto_publish and p.updated_by is not null \
           and now() >= e.start_time - interval '30 minutes' and now() < e.end_time",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Activated packages whose event has ended — auto-deactivate (cancel live rows) + archive.
pub async fn ended_activated_packages(pool: &PgPool) -> Result<Vec<SchedulablePackage>, ApiError> {
    sqlx::query_as::<_, SchedulablePackage>(
        "select p.id, p.event_id, p.updated_by from events.tmi_package p \
         join events.event e on e.id = p.event_id \
         where p.status = 'activated' and p.updated_by is not null and now() >= e.end_time",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The event's debrief notes + who last edited them (display name), if written.
pub async fn get_debrief(
    pool: &PgPool,
    event_id: i64,
) -> Result<Option<(String, Option<String>, DateTime<Utc>)>, ApiError> {
    sqlx::query_as::<_, (String, Option<String>, DateTime<Utc>)>(
        "select d.notes, u.display_name, d.updated_at \
         from events.event_debrief d \
         left join identity.users u on u.id = d.updated_by \
         where d.event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Insert or replace an event's debrief notes, stamping the editor + time.
pub async fn upsert_debrief(
    pool: &PgPool,
    event_id: i64,
    notes: &str,
    user_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into events.event_debrief (event_id, notes, updated_by, updated_at) \
         values ($1, $2, $3, now()) \
         on conflict (event_id) do update \
             set notes = excluded.notes, updated_by = excluded.updated_by, updated_at = now()",
    )
    .bind(event_id)
    .bind(notes)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}
