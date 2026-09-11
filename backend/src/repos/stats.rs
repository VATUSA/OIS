//! Persistence for VATSIM stats collection (schema `stats`, migration 0039). The collector
//! (`feed::stats`) converts each feed tick into the decoupled row structs below and calls the
//! batched upserts here; the compaction job calls the age/prune helpers. Ported from the standalone
//! `stats` ingester, adapted to OIS's `ApiError` + plain-Postgres retention.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};

use crate::errors::ApiError;
use crate::feed::winds::Winds;
use crate::models::{
    DelayGroup, DelaySummary, KeyCountBody, NetworkPointBody, StatsFlightDetail, StatsFlightSummary,
};

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

/// A completed flight leg (departure taxi-out or arrival transit) for `stats.flight_leg`.
pub struct FlightLegRow {
    pub kind: &'static str,
    pub airport: String,
    pub callsign: String,
    pub cid: i32,
    pub aircraft: Option<String>,
    pub runway: Option<String>,
    pub procedure: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub duration_sec: i32,
}

/// Persist completed flight legs (few per tick).
pub async fn insert_flight_legs(pool: &PgPool, rows: &[FlightLegRow]) -> Result<(), ApiError> {
    for chunk in rows.chunks(1000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "insert into stats.flight_leg \
             (kind, airport, callsign, cid, aircraft, runway, procedure, start_time, end_time, duration_sec) ",
        );
        qb.push_values(chunk, |mut b, r| {
            b.push_bind(r.kind)
                .push_bind(&r.airport)
                .push_bind(&r.callsign)
                .push_bind(r.cid)
                .push_bind(&r.aircraft)
                .push_bind(&r.runway)
                .push_bind(&r.procedure)
                .push_bind(r.start)
                .push_bind(r.end)
                .push_bind(r.duration_sec);
        });
        qb.build().execute(pool).await.map_err(db)?;
    }
    Ok(())
}

/// Drop flight legs older than `before` (retention).
pub async fn prune_flight_legs(pool: &PgPool, before: DateTime<Utc>) -> Result<u64, ApiError> {
    let res = sqlx::query("delete from stats.flight_leg where end_time < $1")
        .bind(before)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(res.rows_affected())
}

/// Aggregate expression shared by every delay grouping ($1..$5 = kind, since, airport, runway, proc).
const DELAY_AGG: &str = "count(*)::bigint as n, \
    round(avg(duration_sec))::bigint as avg_sec, \
    percentile_cont(0.5) within group (order by duration_sec)::bigint as median_sec, \
    percentile_cont(0.9) within group (order by duration_sec)::bigint as p90_sec";
const DELAY_FILTER: &str = "kind = $1 and end_time >= $2 \
    and ($3::text is null or airport = $3) \
    and ($4::text is null or runway = $4) \
    and ($5::text is null or procedure = $5)";

#[derive(sqlx::FromRow)]
struct AggRow {
    key: Option<String>,
    n: i64,
    avg_sec: Option<i64>,
    median_sec: Option<i64>,
    p90_sec: Option<i64>,
}

impl From<AggRow> for DelayGroup {
    fn from(r: AggRow) -> Self {
        DelayGroup {
            key: r.key.unwrap_or_default(),
            count: r.n,
            avg_sec: r.avg_sec.unwrap_or(0),
            median_sec: r.median_sec.unwrap_or(0),
            p90_sec: r.p90_sec.unwrap_or(0),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_agg(
    pool: &PgPool,
    sql: &str,
    kind: &str,
    since: DateTime<Utc>,
    airport: Option<&str>,
    runway: Option<&str>,
    procedure: Option<&str>,
) -> Result<Vec<DelayGroup>, ApiError> {
    let rows = sqlx::query_as::<_, AggRow>(sql)
        .bind(kind)
        .bind(since)
        .bind(airport)
        .bind(runway)
        .bind(procedure)
        .fetch_all(pool)
        .await
        .map_err(db)?;
    Ok(rows.into_iter().map(DelayGroup::from).collect())
}

/// Average-delay summary for one leg `kind` since `since`, filtered by the optional airport/runway/
/// procedure. Per-runway and per-procedure breakdowns are computed only when `airport` is set.
#[allow(clippy::too_many_arguments)]
pub async fn delay_summary(
    pool: &PgPool,
    kind: &str,
    airport: Option<&str>,
    runway: Option<&str>,
    procedure: Option<&str>,
    since: DateTime<Utc>,
    window_hours: i64,
) -> Result<DelaySummary, ApiError> {
    let overall_sql =
        format!("select null::text as key, {DELAY_AGG} from stats.flight_leg where {DELAY_FILTER}");
    let by = |dim: &str| {
        format!(
            "select {dim} as key, {DELAY_AGG} from stats.flight_leg \
             where {DELAY_FILTER} and {dim} is not null group by {dim} order by n desc, {dim}"
        )
    };

    let overall = run_agg(pool, &overall_sql, kind, since, airport, runway, procedure)
        .await?
        .into_iter()
        .next()
        .unwrap_or(DelayGroup {
            key: String::new(),
            count: 0,
            avg_sec: 0,
            median_sec: 0,
            p90_sec: 0,
        });
    let by_airport = run_agg(
        pool,
        &by("airport"),
        kind,
        since,
        airport,
        runway,
        procedure,
    )
    .await?;
    let (by_runway, by_procedure) = if airport.is_some() {
        (
            run_agg(pool, &by("runway"), kind, since, airport, runway, procedure).await?,
            run_agg(
                pool,
                &by("procedure"),
                kind,
                since,
                airport,
                runway,
                procedure,
            )
            .await?,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(DelaySummary {
        kind: kind.to_string(),
        window_hours,
        overall,
        by_airport,
        by_runway,
        by_procedure,
    })
}

/// Record a flight-plan revision for any flight whose plan changed since its last recorded revision —
/// keyed off VATSIM's `revision_id`, falling back to a route/dep/arr content compare when it's null.
/// Set-based against the incoming tick batch (lateral-joined to each session's latest revision), so
/// unchanged flights insert nothing (the common case). See migration 0044.
pub async fn insert_flight_plan_revisions(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[FlightRow],
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    for chunk in rows.chunks(2000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "insert into stats.flight_plan (session_id, effective_from, revision_id, flight_rules, \
             departure, arrival, alternate, aircraft_short, aircraft_faa, cruise_alt, deptime, \
             enroute_time, route, remarks) select v.session_id, ",
        );
        qb.push_bind(now);
        // `push_values` emits the `VALUES` keyword itself, so open only the subquery paren here — a
        // literal `values` before it would double the keyword (a syntax error).
        qb.push(
            ", v.revision_id, v.flight_rules, v.departure, v.arrival, v.alternate, v.aircraft_short, \
             v.aircraft_faa, v.cruise_alt, v.deptime, v.enroute_time, v.route, v.remarks from (",
        );
        qb.push_values(chunk, |mut b, f| {
            b.push_bind(f.session_id)
                .push_bind(f.revision_id)
                .push_bind(&f.flight_rules)
                .push_bind(&f.departure)
                .push_bind(&f.arrival)
                .push_bind(&f.alternate)
                .push_bind(&f.aircraft_short)
                .push_bind(&f.aircraft_faa)
                .push_bind(f.cruise_alt)
                .push_bind(&f.deptime)
                .push_bind(&f.enroute_time)
                .push_bind(&f.route)
                .push_bind(&f.remarks);
        });
        qb.push(
            ") as v(session_id, revision_id, flight_rules, departure, arrival, alternate, \
             aircraft_short, aircraft_faa, cruise_alt, deptime, enroute_time, route, remarks) \
             left join lateral ( \
                 select 1 as present, fp.revision_id, fp.route, fp.departure, fp.arrival \
                 from stats.flight_plan fp \
                 where fp.session_id = v.session_id \
                 order by fp.effective_from desc limit 1 \
             ) last on true \
             where last.present is null \
                or v.revision_id is distinct from last.revision_id \
                or (v.revision_id is null and (v.route, v.departure, v.arrival) \
                    is distinct from (last.route, last.departure, last.arrival)) \
             on conflict (session_id, effective_from) do nothing",
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

/// Save an already-elapsed `[start, end)` window as a capture directly, bypassing the open/close
/// lifecycle — used to keep a window after the fact rather than while it's being recorded live.
/// `relax_scope` is false: the data already exists, so there's nothing left for the live collector
/// to relax scope for.
pub async fn save_capture_window(
    pool: &PgPool,
    event_id: Option<i64>,
    label: &str,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    created_by: Option<&str>,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into stats.capture (event_id, label, start_time, end_time, status, relax_scope, created_by)
         values ($1, $2, $3, $4, 'saved', false, $5) returning id",
    )
    .bind(event_id)
    .bind(label)
    .bind(start_time)
    .bind(end_time)
    .bind(created_by)
    .fetch_one(pool)
    .await
    .map_err(db)
}

/// Whether any raw `stats.position` row falls in `[from, to)` — used to reject saving a capture
/// over a window that compaction has already thinned past the point of being worth keeping.
pub async fn has_positions_in(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        "select exists(select 1 from stats.position where ts >= $1 and ts < $2)",
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .map_err(db)
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

/// Current disk usage and a naive, no-further-compaction projection for the `stats` schema —
/// headline numbers only (it deliberately does not model the compaction ladder's ongoing
/// thinning, so the projections are an upper bound, not a precise forecast).
pub async fn storage_forecast(pool: &PgPool) -> Result<StorageForecastBody, ApiError> {
    let total_bytes: i64 = sqlx::query_scalar(
        "select coalesce(sum(pg_total_relation_size(format('stats.%I', tablename)::regclass)), 0)
         from pg_tables where schemaname = 'stats'",
    )
    .fetch_one(pool)
    .await
    .map_err(db)?;
    let position_bytes: i64 =
        sqlx::query_scalar("select pg_total_relation_size('stats.position'::regclass)")
            .fetch_one(pool)
            .await
            .map_err(db)?;
    let position_rows: i64 = sqlx::query_scalar("select count(*) from stats.position")
        .fetch_one(pool)
        .await
        .map_err(db)?;
    let daily_ingest_rows: i64 = sqlx::query_scalar(
        "select count(*) from stats.position where ts >= now() - interval '1 day'",
    )
    .fetch_one(pool)
    .await
    .map_err(db)?;

    let avg_row_bytes = position_bytes / position_rows.max(1);
    let daily_growth_bytes = daily_ingest_rows * avg_row_bytes;
    Ok(StorageForecastBody {
        total_bytes,
        position_bytes,
        daily_ingest_rows,
        daily_growth_bytes,
        projected_30d_bytes: total_bytes + daily_growth_bytes * 30,
        projected_90d_bytes: total_bytes + daily_growth_bytes * 90,
    })
}

/// The most recent capture (open or saved) tied to an event — the window stats are generated over.
pub async fn latest_capture_for_event(
    pool: &PgPool,
    event_id: i64,
) -> Result<Option<CaptureRow>, ApiError> {
    sqlx::query_as::<_, CaptureRow>(&format!(
        "{CAPTURE_SELECT} where event_id = $1 and status <> 'discarded' \
         order by start_time desc limit 1"
    ))
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(db)
}

/// Close every open capture for an event (mark `saved` with the given end time). Returns count.
pub async fn close_open_event_captures(
    pool: &PgPool,
    event_id: i64,
    end_time: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let res = sqlx::query(
        "update stats.capture set end_time = $2, status = 'saved' \
         where event_id = $1 and status = 'open'",
    )
    .bind(event_id)
    .bind(end_time)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(res.rows_affected())
}

// --- per-event capture config ------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventCaptureRow {
    pub event_id: i64,
    pub enabled: bool,
    pub pre_minutes: i32,
    pub post_minutes: i32,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

pub async fn get_event_capture(
    pool: &PgPool,
    event_id: i64,
) -> Result<Option<EventCaptureRow>, ApiError> {
    sqlx::query_as::<_, EventCaptureRow>(
        "select ec.event_id, ec.enabled, ec.pre_minutes, ec.post_minutes, ec.updated_at, \
            u.display_name as updated_by \
         from stats.event_capture ec left join identity.users u on u.id = ec.updated_by \
         where ec.event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(db)
}

pub async fn upsert_event_capture(
    pool: &PgPool,
    event_id: i64,
    enabled: bool,
    pre_minutes: i32,
    post_minutes: i32,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into stats.event_capture (event_id, enabled, pre_minutes, post_minutes, updated_by)
         values ($1, $2, $3, $4, $5)
         on conflict (event_id) do update set
             enabled = excluded.enabled,
             pre_minutes = excluded.pre_minutes,
             post_minutes = excluded.post_minutes,
             updated_by = excluded.updated_by",
    )
    .bind(event_id)
    .bind(enabled)
    .bind(pre_minutes)
    .bind(post_minutes)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}

/// Enabled event captures joined with their event window + whether a capture is currently open.
/// Drives the scheduler (`jobs::spawn_capture_scheduler`).
#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleRow {
    pub event_id: i64,
    pub title: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub pre_minutes: i32,
    pub post_minutes: i32,
    pub open_capture_id: Option<String>,
}

pub async fn list_capture_schedule(pool: &PgPool) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "select e.id as event_id, e.title, e.start_time, e.end_time, \
            ec.pre_minutes, ec.post_minutes, \
            (select c.id from stats.capture c where c.event_id = e.id and c.status = 'open' limit 1) \
                as open_capture_id \
         from stats.event_capture ec join events.event e on e.id = ec.event_id \
         where ec.enabled",
    )
    .fetch_all(pool)
    .await
    .map_err(db)
}

// --- event debrief stats (per featured airport + combined, over a capture window) --------------
//
// A `stats.flight` "overlaps" the window when `first_seen <= to and last_seen >= from`. The window
// bounds are always bound as `$1` (from) / `$2` (to), the featured ICAOs as `$3`.

#[derive(Debug, sqlx::FromRow)]
pub struct KeyCount {
    pub key: Option<String>,
    pub count: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct AirportBreakdown {
    pub icao: String,
    pub arrivals: i64,
    pub departures: i64,
    pub unique_pilots: i64,
}

/// Arrivals, departures, and distinct pilots for each featured airport during the window. A pilot
/// that both arrived at and departed from the same field (turnaround) is one unique pilot but two
/// movements.
pub async fn event_airport_breakdown(
    pool: &PgPool,
    icaos: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<AirportBreakdown>, ApiError> {
    sqlx::query_as::<_, AirportBreakdown>(
        "select icao,
                count(*) filter (where kind = 'arr') as arrivals,
                count(*) filter (where kind = 'dep') as departures,
                count(distinct cid) as unique_pilots
         from (
            select arrival as icao, cid, 'arr' as kind from stats.flight
              where arrival = any($3) and status <> 'prefiled' and first_seen <= $2 and last_seen >= $1
            union all
            select departure as icao, cid, 'dep' as kind from stats.flight
              where departure = any($3) and status <> 'prefiled' and first_seen <= $2 and last_seen >= $1
         ) t
         group by icao order by (count(*)) desc",
    )
    .bind(from)
    .bind(to)
    .bind(icaos)
    .fetch_all(pool)
    .await
    .map_err(db)
}

#[derive(Debug, sqlx::FromRow)]
pub struct AirportKeyCount {
    pub icao: String,
    pub key: Option<String>,
    pub count: i64,
}

/// Top aircraft types by operations (arrivals + departures) at each featured airport — up to
/// `per_airport` per field, ordered busiest first.
pub async fn event_airport_top_aircraft(
    pool: &PgPool,
    icaos: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    per_airport: i64,
) -> Result<Vec<AirportKeyCount>, ApiError> {
    sqlx::query_as::<_, AirportKeyCount>(
        "select icao, key, count from (
            select icao, aircraft_short as key, count(*) as count,
                   row_number() over (partition by icao order by count(*) desc, aircraft_short) as rn
            from (
               select arrival as icao, aircraft_short from stats.flight
                 where arrival = any($3) and status <> 'prefiled' and aircraft_short is not null
                   and first_seen <= $2 and last_seen >= $1
               union all
               select departure as icao, aircraft_short from stats.flight
                 where departure = any($3) and status <> 'prefiled' and aircraft_short is not null
                   and first_seen <= $2 and last_seen >= $1
            ) f
            group by icao, aircraft_short
         ) r where rn <= $4 order by icao, count desc",
    )
    .bind(from)
    .bind(to)
    .bind(icaos)
    .bind(per_airport)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Distinct pilots (CIDs) across all featured airports combined — a pilot flying between two
/// featured fields counts once.
pub async fn event_combined_unique_pilots(
    pool: &PgPool,
    icaos: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, i64>(
        "select count(distinct cid) from stats.flight
         where (departure = any($3) or arrival = any($3))
           and status <> 'prefiled' and first_seen <= $2 and last_seen >= $1",
    )
    .bind(from)
    .bind(to)
    .bind(icaos)
    .fetch_one(pool)
    .await
    .map_err(db)
}

/// Top aircraft types by operations across all featured airports combined.
pub async fn event_combined_top_aircraft(
    pool: &PgPool,
    icaos: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<KeyCount>, ApiError> {
    sqlx::query_as::<_, KeyCount>(
        "select key, count(*) as count from (
            select aircraft_short as key from stats.flight
              where arrival = any($3) and status <> 'prefiled' and aircraft_short is not null
                and first_seen <= $2 and last_seen >= $1
            union all
            select aircraft_short as key from stats.flight
              where departure = any($3) and status <> 'prefiled' and aircraft_short is not null
                and first_seen <= $2 and last_seen >= $1
         ) t group by key order by count(*) desc limit 8",
    )
    .bind(from)
    .bind(to)
    .bind(icaos)
    .fetch_all(pool)
    .await
    .map_err(db)
}

// --- stats read API queries --------------------------------------------------------------------

/// Hourly network totals from `stats.snapshot` (computed on read — no continuous aggregate).
pub async fn network_history(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<NetworkPointBody>, ApiError> {
    sqlx::query_as::<_, NetworkPointBody>(
        "select date_trunc('hour', ts) as hour,
                avg(pilots)::int as avg_pilots,
                max(pilots) as peak_pilots,
                avg(controllers)::int as avg_controllers,
                max(connected_clients) as peak_clients
         from stats.snapshot where ts between $1 and $2
         group by 1 order by 1",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Busiest airports by departures + arrivals (completed/active flights).
pub async fn airports_top(pool: &PgPool, limit: i64) -> Result<Vec<KeyCountBody>, ApiError> {
    sqlx::query_as::<_, KeyCountBody>(
        "select ap as key, count(*) as count from (
            select departure as ap from stats.flight where status <> 'prefiled' and departure is not null
            union all
            select arrival as ap from stats.flight where status <> 'prefiled' and arrival is not null
         ) t group by ap order by count desc limit $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Departure + arrival counts for an airport.
pub async fn airport_counts(pool: &PgPool, icao: &str) -> Result<(i64, i64), ApiError> {
    let dep = sqlx::query_scalar::<_, i64>(
        "select count(*) from stats.flight where departure = $1 and status <> 'prefiled'",
    )
    .bind(icao)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    let arr = sqlx::query_scalar::<_, i64>(
        "select count(*) from stats.flight where arrival = $1 and status <> 'prefiled'",
    )
    .bind(icao)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    Ok((dep, arr))
}

/// Top aircraft to/from an airport.
pub async fn airport_top_aircraft(
    pool: &PgPool,
    icao: &str,
) -> Result<Vec<KeyCountBody>, ApiError> {
    sqlx::query_as::<_, KeyCountBody>(
        "select aircraft_short as key, count(*) as count from stats.flight
         where (departure = $1 or arrival = $1) and status <> 'prefiled' and aircraft_short is not null
         group by 1 order by 2 desc limit 10",
    )
    .bind(icao)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Top destinations from an airport (departures) or origins into it (arrivals).
pub async fn airport_top_endpoints(
    pool: &PgPool,
    icao: &str,
    origins: bool,
) -> Result<Vec<KeyCountBody>, ApiError> {
    // `match_col`/`group_col` are fixed internal literals, never user input.
    let (match_col, group_col) = if origins {
        ("arrival", "departure") // origins into this airport
    } else {
        ("departure", "arrival") // destinations from this airport
    };
    let sql = format!(
        "select {group_col} as key, count(*) as count from stats.flight
         where {match_col} = $1 and status <> 'prefiled' and {group_col} is not null
         group by 1 order by 2 desc limit 10"
    );
    sqlx::query_as::<_, KeyCountBody>(&sql)
        .bind(icao)
        .fetch_all(pool)
        .await
        .map_err(db)
}

const FLIGHT_SUMMARY_SELECT: &str = "select session_id, callsign, status, logon_time, departure, \
    arrival, aircraft_short, duration_s, distance_nm from stats.flight";

/// Recent departures (`departure`) or arrivals (`arrival`) at an airport.
pub async fn airport_movements(
    pool: &PgPool,
    icao: &str,
    arrivals: bool,
    limit: i64,
) -> Result<Vec<StatsFlightSummary>, ApiError> {
    let col = if arrivals { "arrival" } else { "departure" };
    let sql = format!(
        "{FLIGHT_SUMMARY_SELECT} where {col} = $1 and status <> 'prefiled' \
         order by logon_time desc limit $2"
    );
    sqlx::query_as::<_, StatsFlightSummary>(&sql)
        .bind(icao)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(db)
}

/// A member's recent flights.
pub async fn member_flights(
    pool: &PgPool,
    cid: i32,
    limit: i64,
) -> Result<Vec<StatsFlightSummary>, ApiError> {
    sqlx::query_as::<_, StatsFlightSummary>(&format!(
        "{FLIGHT_SUMMARY_SELECT} where cid = $1 and status <> 'prefiled' \
         order by logon_time desc limit $2"
    ))
    .bind(cid)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(db)
}

pub async fn flight_detail(
    pool: &PgPool,
    session_id: i64,
) -> Result<Option<StatsFlightDetail>, ApiError> {
    sqlx::query_as::<_, StatsFlightDetail>(
        "select session_id, cid, callsign, server, status, logon_time, first_seen, last_seen,
                departure, arrival, alternate, aircraft_short, cruise_alt, route,
                duration_s, distance_nm, max_altitude, max_groundspeed
         from stats.flight where session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(db)
}

/// The flight's plan revisions, oldest first (empty for pre-0044 flights; one entry = never amended).
pub async fn flight_plan_history(
    pool: &PgPool,
    session_id: i64,
) -> Result<Vec<crate::models::FlightPlanRevisionBody>, ApiError> {
    sqlx::query_as(
        "select effective_from, departure, arrival, aircraft_short, route
         from stats.flight_plan where session_id = $1 order by effective_from",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Raw 15s track (ts, lat, lon, altitude, groundspeed, heading) for a flight, oldest first.
pub async fn flight_track_raw(
    pool: &PgPool,
    session_id: i64,
) -> Result<Vec<(DateTime<Utc>, f32, f32, i32, i16, i16)>, ApiError> {
    sqlx::query_as::<_, (DateTime<Utc>, f32, f32, i32, i16, i16)>(
        "select ts, lat, lon, altitude, groundspeed, heading from stats.position
         where session_id = $1 order by ts",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// The stored simplified path for a flight (Tier-1), if any.
pub async fn flight_path_simplified(
    pool: &PgPool,
    session_id: i64,
) -> Result<Option<Value>, ApiError> {
    let row = sqlx::query_scalar::<_, Option<Value>>(
        "select path_simplified from stats.flight where session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(db)?;
    Ok(row.flatten())
}

// --- capture replay ----------------------------------------------------------------------------

use crate::models::{CaptureSummaryBody, StorageForecastBody};

/// Replayable captures (open or saved), newest first, with the tied event's title.
pub async fn list_replayable_captures(pool: &PgPool) -> Result<Vec<CaptureSummaryBody>, ApiError> {
    sqlx::query_as::<_, CaptureSummaryBody>(
        "select c.id, c.event_id, e.title as event_title, c.label, c.start_time, c.end_time, c.status
         from stats.capture c left join events.event e on e.id = c.event_id
         where c.status in ('open', 'saved')
         order by c.start_time desc",
    )
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// A single capture by id.
pub async fn capture_get(pool: &PgPool, id: &str) -> Result<Option<CaptureRow>, ApiError> {
    sqlx::query_as::<_, CaptureRow>(&format!("{CAPTURE_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)
}

/// A single capture by id, in the joined summary shape (with the tied event's title) — the
/// response shape for saving a new capture.
pub async fn capture_summary_get(
    pool: &PgPool,
    id: &str,
) -> Result<Option<CaptureSummaryBody>, ApiError> {
    sqlx::query_as::<_, CaptureSummaryBody>(
        "select c.id, c.event_id, e.title as event_title, c.label, c.start_time, c.end_time, c.status
         from stats.capture c left join events.event e on e.id = c.event_id
         where c.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(db)
}

/// One thinned position sample for replay (`t` = seconds from the window start).
#[derive(sqlx::FromRow)]
pub struct ReplaySample {
    pub session_id: i64,
    pub t: f64,
    pub lat: f32,
    pub lon: f32,
    pub alt: i32,
    pub heading: i16,
    pub gs: i16,
}

/// Every flight's positions in the chunk `[chunk_from, chunk_to)`, thinned to one sample per
/// `step_s`-second bucket per flight. `origin` is the window start, so `t` is seconds from the window
/// start and stays consistent across chunks (the bucket key uses absolute epoch/step, so buckets
/// align regardless of chunk boundaries). Ordered by flight then time for one-pass grouping.
///
/// Progressive replay fetches this per chunk, so the DB only ever scans the requested sub-window —
/// not the whole (possibly multi-day) window at once.
pub async fn replay_positions(
    pool: &PgPool,
    origin: DateTime<Utc>,
    chunk_from: DateTime<Utc>,
    chunk_to: DateTime<Utc>,
    step_s: i64,
) -> Result<Vec<ReplaySample>, ApiError> {
    // One sample per (session, step-second bucket): DISTINCT ON keeps the earliest row in each
    // bucket. Its ORDER BY already emits rows grouped by session and ascending in ts (the bucket is
    // monotonic in ts), which is exactly what the handler needs to fold into per-flight tracks — so
    // there's no outer sort. Half-open `[chunk_from, chunk_to)` so adjacent chunks never double-count.
    sqlx::query_as::<_, ReplaySample>(
        "select distinct on (session_id, floor(extract(epoch from ts) / $4)::bigint)
                session_id,
                extract(epoch from (ts - $1))::float8 as t,
                lat, lon, altitude as alt, heading, groundspeed as gs
         from stats.position
         where ts >= $2 and ts < $3
         order by session_id, floor(extract(epoch from ts) / $4)::bigint, ts",
    )
    .bind(origin)
    .bind(chunk_from)
    .bind(chunk_to)
    .bind(step_s)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Callsign + plan basics for a set of flights (for replay labels).
#[allow(clippy::type_complexity)]
pub async fn flights_meta(
    pool: &PgPool,
    ids: &[i64],
) -> Result<
    Vec<(
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
    ApiError,
> {
    sqlx::query_as(
        "select session_id, callsign, departure, arrival, aircraft_short, route
         from stats.flight where session_id = any($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Flight-plan revisions relevant to a replay `[from, to]`: per session, the latest revision in force
/// at/before `from` (the plan open with) plus every revision that took effect within the window.
/// Ordered by session then effective time. Empty for sessions with no recorded revisions (pre-0044
/// captures) — the caller falls back to `flights_meta`.
#[allow(clippy::type_complexity)]
pub async fn flight_plan_revisions(
    pool: &PgPool,
    ids: &[i64],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<
    Vec<(
        i64,
        DateTime<Utc>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
    ApiError,
> {
    // Each UNION arm is parenthesized: the first carries its own ORDER BY for `distinct on`, which
    // Postgres only allows on a parenthesized union arm (a bare ORDER BY would bind to the whole union).
    sqlx::query_as(
        "select session_id, effective_from, departure, arrival, aircraft_short, route from (
             (select distinct on (session_id)
                 session_id, effective_from, departure, arrival, aircraft_short, route
             from stats.flight_plan
             where session_id = any($1) and effective_from <= $2
             order by session_id, effective_from desc)
             union all
             (select session_id, effective_from, departure, arrival, aircraft_short, route
             from stats.flight_plan
             where session_id = any($1) and effective_from > $2 and effective_from <= $3)
         ) x
         order by session_id, effective_from",
    )
    .bind(ids)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)
}

// --- winds-aloft snapshots (historical ETA accuracy) ------------------------------------------

/// Persist one winds snapshot at `ts` (idempotent per timestamp).
pub async fn upsert_winds(pool: &PgPool, ts: DateTime<Utc>, winds: &Winds) -> Result<(), ApiError> {
    sqlx::query("insert into stats.winds (ts, data) values ($1, $2) on conflict (ts) do nothing")
        .bind(ts)
        .bind(sqlx::types::Json(winds))
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

/// The most recent winds snapshot at or before `at` (None when nothing was captured yet — the
/// caller falls back to still air).
pub async fn winds_at(pool: &PgPool, at: DateTime<Utc>) -> Result<Option<Winds>, ApiError> {
    let row = sqlx::query_scalar::<_, sqlx::types::Json<Winds>>(
        "select data from stats.winds where ts <= $1 order by ts desc limit 1",
    )
    .bind(at)
    .fetch_optional(pool)
    .await
    .map_err(db)?;
    Ok(row.map(|j| j.0))
}

/// Drop winds snapshots older than `before`, except any inside an open/saved capture window.
pub async fn prune_winds(pool: &PgPool, before: DateTime<Utc>) -> Result<u64, ApiError> {
    let res = sqlx::query(
        "delete from stats.winds w where w.ts < $1 and not exists (\
            select 1 from stats.capture c where c.status in ('open', 'saved') \
            and w.ts >= c.start_time and w.ts < coalesce(c.end_time, now()))",
    )
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
