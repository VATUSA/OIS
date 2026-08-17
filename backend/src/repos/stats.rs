//! Persistence for VATSIM stats collection (schema `stats`, migration 0039). The collector
//! (`feed::stats`) converts each feed tick into the decoupled row structs below and calls the
//! batched upserts here; the compaction job calls the age/prune helpers. Ported from the standalone
//! `stats` ingester, adapted to OIS's `ApiError` + plain-Postgres retention.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};

use crate::errors::ApiError;

// --- decoupled row inputs (the collector fills these from feed structs) ---------------------

pub struct FlightRow {
    pub session_id: i64,
    pub cid: i32,
    pub callsign: String,
    pub server: Option<String>,
    pub logon_time: DateTime<Utc>,
    pub flight_rules: Option<String>,
    pub departure: Option<String>,
    pub arrival: Option<String>,
    pub alternate: Option<String>,
    pub aircraft_short: Option<String>,
    pub aircraft_faa: Option<String>,
    pub cruise_tas: Option<i32>,
    pub cruise_alt: Option<i32>,
    pub deptime: Option<String>,
    pub enroute_time: Option<String>,
    pub route: Option<String>,
    pub remarks: Option<String>,
    pub revision_id: Option<i32>,
}

pub struct PrefileRow {
    pub session_id: i64,
    pub cid: i32,
    pub callsign: String,
    pub departure: Option<String>,
    pub arrival: Option<String>,
    pub alternate: Option<String>,
    pub aircraft_short: Option<String>,
    pub route: Option<String>,
    pub remarks: Option<String>,
    pub cruise_alt: Option<i32>,
    pub revision_id: Option<i32>,
}

pub struct ControllerRow {
    pub session_id: i64,
    pub cid: i32,
    pub callsign: String,
    pub frequency: Option<String>,
    pub facility: Option<i32>,
    pub rating: Option<i32>,
    pub server: Option<String>,
    pub visual_range: Option<i32>,
    pub atis_code: Option<String>,
    pub logon_time: DateTime<Utc>,
    pub is_atis: bool,
}

pub struct PositionRow {
    pub session_id: i64,
    pub lat: f32,
    pub lon: f32,
    pub altitude: i32,
    pub groundspeed: i16,
    pub heading: i16,
    pub transponder: Option<String>,
    pub qnh_mb: Option<i16>,
}

pub struct SnapshotCounts {
    pub connected_clients: i32,
    pub unique_users: i32,
    pub pilots: i32,
    pub controllers: i32,
    pub atis: i32,
    pub prefiles: i32,
}

// --- crash recovery ----------------------------------------------------------------------------

/// Reload live flights on startup so they aren't re-opened as new sessions.
/// Returns `(session_id, last_seen, revision_id)`.
pub async fn list_active_flights(
    pool: &PgPool,
) -> Result<Vec<(i64, DateTime<Utc>, Option<i32>)>, ApiError> {
    sqlx::query_as::<_, (i64, DateTime<Utc>, Option<i32>)>(
        "select session_id, last_seen, revision_id from stats.flight where status = 'active'",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

// --- batched writes (one transaction per tick) -------------------------------------------------

/// Batched member upsert (deduped by cid upstream). 3 cols/row.
pub async fn upsert_members(
    tx: &mut Transaction<'_, Postgres>,
    names: &[(i32, String)],
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    for chunk in names.chunks(5000) {
        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new("insert into stats.member (cid, name, last_seen) ");
        qb.push_values(chunk, |mut b, (cid, name)| {
            b.push_bind(*cid).push_bind(name).push_bind(now);
        });
        qb.push(
            " on conflict (cid) do update set name = excluded.name, last_seen = excluded.last_seen",
        );
        qb.build().execute(&mut **tx).await.map_err(db)?;
    }
    Ok(())
}

/// Batched flight upsert (status='active'). 21 cols/row → chunk under the 65535-param cap.
pub async fn upsert_flights(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[FlightRow],
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    for chunk in rows.chunks(2000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "insert into stats.flight (
                session_id, cid, callsign, server, logon_time, first_seen, last_seen, status,
                flight_rules, departure, arrival, alternate, aircraft_short, aircraft_faa,
                cruise_tas, cruise_alt, deptime, enroute_time, route, remarks, revision_id) ",
        );
        qb.push_values(chunk, |mut b, f| {
            b.push_bind(f.session_id)
                .push_bind(f.cid)
                .push_bind(&f.callsign)
                .push_bind(&f.server)
                .push_bind(f.logon_time)
                .push_bind(now)
                .push_bind(now)
                .push_bind("active")
                .push_bind(&f.flight_rules)
                .push_bind(&f.departure)
                .push_bind(&f.arrival)
                .push_bind(&f.alternate)
                .push_bind(&f.aircraft_short)
                .push_bind(&f.aircraft_faa)
                .push_bind(f.cruise_tas)
                .push_bind(f.cruise_alt)
                .push_bind(&f.deptime)
                .push_bind(&f.enroute_time)
                .push_bind(&f.route)
                .push_bind(&f.remarks)
                .push_bind(f.revision_id);
        });
        qb.push(
            " on conflict (session_id) do update set
                last_seen = excluded.last_seen,
                status = 'active',
                departure = excluded.departure,
                arrival = excluded.arrival,
                alternate = excluded.alternate,
                route = excluded.route,
                remarks = excluded.remarks,
                cruise_alt = excluded.cruise_alt,
                revision_id = excluded.revision_id",
        );
        qb.build().execute(&mut **tx).await.map_err(db)?;
    }
    Ok(())
}

/// Batched prefile upsert (provisional flights, no positions). 11 cols/row.
pub async fn upsert_prefiles(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[PrefileRow],
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    for chunk in rows.chunks(3000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "insert into stats.flight (
                session_id, cid, callsign, logon_time, first_seen, last_seen, status,
                departure, arrival, alternate, aircraft_short, route, remarks, cruise_alt, revision_id) ",
        );
        qb.push_values(chunk, |mut b, p| {
            b.push_bind(p.session_id)
                .push_bind(p.cid)
                .push_bind(&p.callsign)
                .push_bind(now)
                .push_bind(now)
                .push_bind(now)
                .push_bind("prefiled")
                .push_bind(&p.departure)
                .push_bind(&p.arrival)
                .push_bind(&p.alternate)
                .push_bind(&p.aircraft_short)
                .push_bind(&p.route)
                .push_bind(&p.remarks)
                .push_bind(p.cruise_alt)
                .push_bind(p.revision_id);
        });
        qb.push(
            " on conflict (session_id) do update set
                last_seen = excluded.last_seen,
                departure = excluded.departure,
                arrival = excluded.arrival,
                route = excluded.route,
                cruise_alt = excluded.cruise_alt
             where stats.flight.status = 'prefiled'",
        );
        qb.build().execute(&mut **tx).await.map_err(db)?;
    }
    Ok(())
}

/// Batched controller/ATIS upsert. 13 cols/row.
pub async fn upsert_controllers(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[ControllerRow],
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    for chunk in rows.chunks(3000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "insert into stats.controller_session (
                session_id, cid, callsign, frequency, facility, rating, server,
                visual_range, atis_code, logon_time, first_seen, last_seen, is_atis) ",
        );
        qb.push_values(chunk, |mut b, c| {
            b.push_bind(c.session_id)
                .push_bind(c.cid)
                .push_bind(&c.callsign)
                .push_bind(&c.frequency)
                .push_bind(c.facility)
                .push_bind(c.rating)
                .push_bind(&c.server)
                .push_bind(c.visual_range)
                .push_bind(&c.atis_code)
                .push_bind(c.logon_time)
                .push_bind(now)
                .push_bind(now)
                .push_bind(c.is_atis);
        });
        qb.push(
            " on conflict (session_id) do update set
                last_seen = excluded.last_seen,
                atis_code = excluded.atis_code",
        );
        qb.build().execute(&mut **tx).await.map_err(db)?;
    }
    Ok(())
}

/// Batched position insert (all share the tick `now`). 9 cols/row.
pub async fn insert_positions(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[PositionRow],
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    for chunk in rows.chunks(5000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, heading, transponder, qnh_mb) ",
        );
        qb.push_values(chunk, |mut b, p| {
            b.push_bind(p.session_id)
                .push_bind(now)
                .push_bind(p.lat)
                .push_bind(p.lon)
                .push_bind(p.altitude)
                .push_bind(p.groundspeed)
                .push_bind(p.heading)
                .push_bind(&p.transponder)
                .push_bind(p.qnh_mb);
        });
        qb.build().execute(&mut **tx).await.map_err(db)?;
    }
    Ok(())
}

/// One network-totals row for the tick.
pub async fn insert_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    now: DateTime<Utc>,
    c: &SnapshotCounts,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into stats.snapshot (ts, connected_clients, unique_users, pilots, controllers, atis, prefiles)
         values ($1,$2,$3,$4,$5,$6,$7) on conflict (ts) do nothing",
    )
    .bind(now)
    .bind(c.connected_clients)
    .bind(c.unique_users)
    .bind(c.pilots)
    .bind(c.controllers)
    .bind(c.atis)
    .bind(c.prefiles)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

// --- session close -----------------------------------------------------------------------------

/// The stored raw track of a flight (ordered), for the on-close summary.
pub async fn fetch_flight_track(
    pool: &PgPool,
    sid: i64,
) -> Result<Vec<(DateTime<Utc>, f32, f32, i32, i16)>, ApiError> {
    sqlx::query_as::<_, (DateTime<Utc>, f32, f32, i32, i16)>(
        "select ts, lat, lon, altitude, groundspeed from stats.position
         where session_id = $1 order by ts",
    )
    .bind(sid)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Mark a flight completed with its computed summary (all `None` when it had no track).
#[allow(clippy::too_many_arguments)]
pub async fn set_flight_completed(
    pool: &PgPool,
    sid: i64,
    duration_s: Option<i32>,
    distance_nm: Option<f32>,
    max_altitude: Option<i32>,
    max_groundspeed: Option<i32>,
    path_simplified: Option<&Value>,
) -> Result<(), ApiError> {
    sqlx::query(
        "update stats.flight set
            status = 'completed',
            duration_s = $2,
            distance_nm = $3,
            max_altitude = $4,
            max_groundspeed = $5,
            path_simplified = $6
         where session_id = $1",
    )
    .bind(sid)
    .bind(duration_s)
    .bind(distance_nm)
    .bind(max_altitude)
    .bind(max_groundspeed)
    .bind(path_simplified.map(sqlx::types::Json))
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}

/// Close a controller session (fill in duration).
pub async fn close_controller(pool: &PgPool, sid: i64) -> Result<(), ApiError> {
    sqlx::query(
        "update stats.controller_session
         set duration_s = extract(epoch from (last_seen - first_seen))::int
         where session_id = $1",
    )
    .bind(sid)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}

// --- capture windows ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CaptureRow {
    pub id: String,
    pub event_id: Option<i64>,
    pub label: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub status: String,
    pub relax_scope: bool,
    pub created_at: DateTime<Utc>,
}

const CAPTURE_SELECT: &str = "select id, event_id, label, start_time, end_time, status, \
    relax_scope, created_at from stats.capture";

/// Whether any capture is currently open (collector relaxes the US scope while true).
pub async fn has_open_capture(pool: &PgPool) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        "select exists(select 1 from stats.capture where status = 'open' and relax_scope)",
    )
    .fetch_one(pool)
    .await
    .map_err(db)
}

pub async fn list_captures(pool: &PgPool) -> Result<Vec<CaptureRow>, ApiError> {
    sqlx::query_as::<_, CaptureRow>(&format!("{CAPTURE_SELECT} order by start_time desc"))
        .fetch_all(pool)
        .await
        .map_err(db)
}

pub async fn create_capture(
    pool: &PgPool,
    event_id: Option<i64>,
    label: &str,
    start_time: DateTime<Utc>,
    created_by: Option<&str>,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into stats.capture (event_id, label, start_time, created_by)
         values ($1, $2, $3, $4) returning id",
    )
    .bind(event_id)
    .bind(label)
    .bind(start_time)
    .bind(created_by)
    .fetch_one(pool)
    .await
    .map_err(db)
}

/// Close a capture: set its end time and mark it `saved` (permanently kept) or `discarded`.
pub async fn close_capture(
    pool: &PgPool,
    id: &str,
    end_time: DateTime<Utc>,
    status: &str,
) -> Result<bool, ApiError> {
    let res = sqlx::query(
        "update stats.capture set end_time = $2, status = $3 where id = $1 and status = 'open'",
    )
    .bind(id)
    .bind(end_time)
    .bind(status)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(res.rows_affected() > 0)
}

// --- compaction / retention (saved-window-aware) ----------------------------------------------

/// A `stats.position` row is protected from compaction while its `ts` falls inside a capture that is
/// still open or has been saved.
const CAPTURE_GUARD: &str = "not exists (select 1 from stats.capture c \
    where c.status in ('open', 'saved') \
    and p.ts >= c.start_time and p.ts < coalesce(c.end_time, now()))";

/// Downsample positions in the age band `[from, to)`: keep only every `keep_every`-th sample per
/// session (ordered by ts), delete the rest — except protected (capture) rows. Returns rows deleted.
pub async fn downsample_positions(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    keep_every: i64,
) -> Result<u64, ApiError> {
    let sql = format!(
        "delete from stats.position p using (
            select ctid, row_number() over (partition by session_id order by ts) as rn
            from stats.position p
            where p.ts >= $1 and p.ts < $2 and {CAPTURE_GUARD}
         ) d
         where p.ctid = d.ctid and (d.rn % $3) <> 0"
    );
    let res = sqlx::query(&sql)
        .bind(from)
        .bind(to)
        .bind(keep_every)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(res.rows_affected())
}

/// Drop all raw positions older than `before`, except protected (capture) rows. Returns rows
/// deleted. Tier-1 simplified tracks on `stats.flight.path_simplified` survive this.
pub async fn prune_positions(pool: &PgPool, before: DateTime<Utc>) -> Result<u64, ApiError> {
    let sql = format!("delete from stats.position p where p.ts < $1 and {CAPTURE_GUARD}");
    let res = sqlx::query(&sql)
        .bind(before)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(res.rows_affected())
}

fn db(e: sqlx::Error) -> ApiError {
    tracing::warn!(error = %e, "stats db error");
    ApiError::Internal
}
