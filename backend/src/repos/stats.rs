//! Persistence for VATSIM stats collection (schema `stats`, migration 0039). The collector
//! (`feed::stats`) converts each feed tick into the decoupled row structs below and calls the
//! batched upserts here; the compaction job calls the age/prune helpers. Ported from the standalone
//! `stats` ingester, adapted to OIS's `ApiError` + plain-Postgres retention.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};

use crate::auth::principal::Attribution;
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
        // Not persistent (#689): `push_values` makes the SQL text depend on the row count, which changes
        // every tick, so a prepared statement here is never reused — and kept, each one is a new entry
        // in this connection's statement cache, for every connection in the pool. sqlx's own note on
        // `push_values` says to do exactly this. The same applies to every batch insert below.
        qb.build()
            .persistent(false)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
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
        qb.build()
            .persistent(false)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
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
        qb.build()
            .persistent(false)
            .execute(pool)
            .await
            .map_err(db)?;
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

/// A completed departure's pushback, start-up, and taxi-out timings for `stats.taxi_observation`
/// (#164 sub-issue C — the raw observations a later per-gate/type/runway estimator learns from).
pub struct TaxiObservationRow {
    pub airport: String,
    pub gate_id: Option<String>,
    pub aircraft: Option<String>,
    pub runway: Option<String>,
    pub pushback_sec: Option<i32>,
    pub startup_sec: Option<i32>,
    pub taxi_sec: i32,
    pub observed_at: DateTime<Utc>,
}

/// Persist completed taxi observations (few per tick).
pub async fn insert_taxi_observations(
    pool: &PgPool,
    rows: &[TaxiObservationRow],
) -> Result<(), ApiError> {
    for chunk in rows.chunks(1000) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "insert into stats.taxi_observation \
             (airport, gate_id, aircraft, runway, pushback_sec, startup_sec, taxi_sec, observed_at) ",
        );
        qb.push_values(chunk, |mut b, r| {
            b.push_bind(&r.airport)
                .push_bind(&r.gate_id)
                .push_bind(&r.aircraft)
                .push_bind(&r.runway)
                .push_bind(r.pushback_sec)
                .push_bind(r.startup_sec)
                .push_bind(r.taxi_sec)
                .push_bind(r.observed_at);
        });
        qb.build()
            .persistent(false)
            .execute(pool)
            .await
            .map_err(db)?;
    }
    Ok(())
}

/// Drop taxi observations older than `before` (retention).
pub async fn prune_taxi_observations(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let res = sqlx::query("delete from stats.taxi_observation where observed_at < $1")
        .bind(before)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(res.rows_affected())
}

/// Every taxi observation for one airport — the sample set `feed::taxi_estimate::estimate` walks
/// its fallback ladder over (#164 sub-issue D). No recency filter: the table is already pruned to
/// `DELAY_LEG_RETAIN_DAYS` by `prune_taxi_observations` above, so every row is already in-window.
pub async fn taxi_samples_for_airport(
    pool: &PgPool,
    airport: &str,
) -> Result<Vec<crate::feed::taxi_estimate::TaxiSample>, ApiError> {
    sqlx::query_as(
        "select gate_id, aircraft, runway, pushback_sec, startup_sec, taxi_sec \
         from stats.taxi_observation where airport = $1",
    )
    .bind(airport)
    .fetch_all(pool)
    .await
    .map_err(db)
}

#[derive(sqlx::FromRow)]
struct AirportTaxiSampleRow {
    airport: String,
    gate_id: Option<String>,
    aircraft: Option<String>,
    runway: Option<String>,
    pushback_sec: Option<i32>,
    startup_sec: Option<i32>,
    taxi_sec: i32,
}

/// Every taxi observation across every airport, grouped by airport — the sample set the DB-less
/// feed subsystem's ground-allowance cache is refreshed from (#164 sub-issue E,
/// `jobs::spawn_taxi_estimate_samples_refresh`).
///
/// **Builds the map and nothing else (#776).** The refresh job runs this every ten minutes while the
/// previous map is still live in its `ArcSwap`, over a table that only grows until the
/// `DELAY_LEG_RETAIN_DAYS` prune. It used to `fetch_all` every row into a `Vec` and then group a
/// second copy, with every airport's `Vec` left at its doubling capacity: on production's 352k rows
/// that was ~108 MB of transient allocation over a ~48 MB map, which pushed each backend replica past
/// its 512 MiB limit ten minutes after it started. Each OOM kill dropped every realtime socket on
/// that replica. Streaming the rows in airport order lets each airport's samples be trimmed as soon as
/// they are complete, so a reload costs one trimmed map (~34 MB on the same rows).
pub async fn load_all_taxi_samples(
    pool: &PgPool,
) -> Result<std::collections::HashMap<String, Vec<crate::feed::taxi_estimate::TaxiSample>>, ApiError>
{
    use crate::feed::taxi_estimate::TaxiSample;
    use futures_util::TryStreamExt;
    use std::collections::{HashMap, hash_map::Entry};

    fn finish(
        by_airport: &mut HashMap<String, Vec<TaxiSample>>,
        airport: String,
        mut samples: Vec<TaxiSample>,
    ) {
        samples.shrink_to_fit();
        match by_airport.entry(airport) {
            Entry::Vacant(slot) => {
                slot.insert(samples);
            }
            // Only if the order ever stopped being contiguous: still correct, just not as lean.
            Entry::Occupied(mut slot) => slot.get_mut().append(&mut samples),
        }
    }

    let mut rows = sqlx::query_as::<_, AirportTaxiSampleRow>(
        "select airport, gate_id, aircraft, runway, pushback_sec, startup_sec, taxi_sec \
         from stats.taxi_observation order by airport",
    )
    .fetch(pool);
    let mut by_airport: HashMap<String, Vec<TaxiSample>> = HashMap::new();
    let mut current: Option<(String, Vec<TaxiSample>)> = None;
    while let Some(r) = rows.try_next().await.map_err(db)? {
        let sample = TaxiSample {
            gate_id: r.gate_id,
            aircraft: r.aircraft,
            runway: r.runway,
            pushback_sec: r.pushback_sec,
            startup_sec: r.startup_sec,
            taxi_sec: r.taxi_sec,
        };
        match &mut current {
            Some((airport, samples)) if *airport == r.airport => samples.push(sample),
            _ => {
                if let Some((airport, samples)) = current.replace((r.airport, vec![sample])) {
                    finish(&mut by_airport, airport, samples);
                }
            }
        }
    }
    if let Some((airport, samples)) = current {
        finish(&mut by_airport, airport, samples);
    }
    Ok(by_airport)
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

/// The per-airport breakdown, paginated — this is the only `delay_summary` grouping that can grow
/// unbounded (every airport with delay data nationally, vs. `by_runway`/`by_procedure` which are
/// scoped to one already-picked airport). Returns the page's rows plus the total distinct-airport
/// count for the same filters.
#[allow(clippy::too_many_arguments)]
async fn by_airport_page(
    pool: &PgPool,
    kind: &str,
    since: DateTime<Utc>,
    airport: Option<&str>,
    runway: Option<&str>,
    procedure: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<DelayGroup>, i64), ApiError> {
    let sql = format!(
        "select airport as key, {DELAY_AGG} from stats.flight_leg \
         where {DELAY_FILTER} and airport is not null group by airport \
         order by n desc, airport limit $6 offset $7"
    );
    let rows = sqlx::query_as::<_, AggRow>(&sql)
        .bind(kind)
        .bind(since)
        .bind(airport)
        .bind(runway)
        .bind(procedure)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(db)?;
    let total_sql = format!(
        "select count(distinct airport) from stats.flight_leg \
         where {DELAY_FILTER} and airport is not null"
    );
    let total: i64 = sqlx::query_scalar(&total_sql)
        .bind(kind)
        .bind(since)
        .bind(airport)
        .bind(runway)
        .bind(procedure)
        .fetch_one(pool)
        .await
        .map_err(db)?;
    Ok((rows.into_iter().map(DelayGroup::from).collect(), total))
}

/// Average-delay summary for one leg `kind` since `since`, filtered by the optional airport/runway/
/// procedure. Per-runway and per-procedure breakdowns are computed only when `airport` is set.
/// `page`/`page_size` paginate `by_airport` only — the other groupings are inherently small.
#[allow(clippy::too_many_arguments)]
pub async fn delay_summary(
    pool: &PgPool,
    kind: &str,
    airport: Option<&str>,
    runway: Option<&str>,
    procedure: Option<&str>,
    since: DateTime<Utc>,
    window_hours: i64,
    page: i64,
    page_size: i64,
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
    let (by_airport, by_airport_total) = by_airport_page(
        pool,
        kind,
        since,
        airport,
        runway,
        procedure,
        page_size,
        (page - 1) * page_size,
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
        by_airport_total,
        page,
        page_size,
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
        qb.build()
            .persistent(false)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
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
        qb.build()
            .persistent(false)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
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
        qb.build()
            .persistent(false)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
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
        qb.build()
            .persistent(false)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
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

/// Marks a capture `discarded`, releasing the positions it was pinning (#432).
///
/// Not a row delete. `CAPTURE_GUARD` (see [`downsample_positions`]) keeps every position inside an
/// `'open'` or `'saved'` window out of compaction, so dropping out of that set is what actually gives
/// the space back — on the next compaction pass, not immediately. Keeping the row also keeps the
/// record that the capture existed, which a hard delete would lose.
///
/// Accepts `'open'` as well as `'saved'`: discarding an ad-hoc capture that is still recording is a
/// coherent thing to want, and leaving it running would keep pinning data. An open *event* capture
/// inside its window is refused by the caller instead — see [`event_capture_is_live`]. Returns
/// `false` when there is no such capture or it was already discarded, so the caller can answer 404
/// rather than pretend.
pub async fn discard_capture(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let res = sqlx::query(
        "update stats.capture set status = 'discarded' \
         where id = $1 and status in ('open', 'saved')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(res.rows_affected() > 0)
}

/// Whether this capture is an event's, still open, and still inside the window the scheduler watches.
///
/// Such a capture cannot usefully be discarded: `list_capture_schedule` derives `open_capture_id`
/// from `status = 'open'`, so discarding it makes the event look like it has no capture, and
/// `capture_scheduler_once`'s `(in_window, None)` arm opens a fresh one on its next pass. The delete
/// would report success, the row would leave the picker, and a new capture would resume pinning the
/// same positions (#432 review).
///
/// The window is the padded one the scheduler uses — `pre_minutes` before the event to `post_minutes`
/// after — because that is the span in which it will reopen.
pub async fn event_capture_is_live(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        "select exists ( \
           select 1 from stats.capture c \
             join events.event e on e.id = c.event_id \
             join stats.event_capture ec on ec.event_id = e.id \
           where c.id = $1 and c.status = 'open' and ec.enabled \
             and now() >= e.start_time - make_interval(mins => ec.pre_minutes) \
             and now() <= e.end_time + make_interval(mins => ec.post_minutes) \
        )",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(db)
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
    by: &Attribution,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into stats.capture (event_id, label, start_time, end_time, status, relax_scope, created_by, created_by_actor)
         values ($1, $2, $3, $4, 'saved', false, $5, $6) returning id",
    )
    .bind(event_id)
    .bind(label)
    .bind(start_time)
    .bind(end_time)
    .bind(&by.user_id)
    .bind(&by.actor_id)
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
    // `sum(bigint)` returns `numeric` in Postgres (headroom against overflow), not `bigint` —
    // cast back or sqlx refuses to decode it into `i64` (this 500'd on every call until caught in
    // review: decode error "Rust type `i64` ... is not compatible with SQL type `NUMERIC`").
    let total_bytes: i64 = sqlx::query_scalar(
        "select coalesce(sum(pg_total_relation_size(format('stats.%I', tablename)::regclass)), 0)::bigint
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
    // Planner's row estimate (from the last ANALYZE/autovacuum), not an exact `count(*)` — this
    // table is the fast-growing one compaction targets, so an exact scan here would be a
    // needlessly heavy full-table read every time this (frequently-polled) endpoint is hit; an
    // estimate is plenty for a headline "average row size" input to a naive projection anyway.
    let position_rows: i64 = sqlx::query_scalar(
        "select greatest(reltuples::bigint, 0) from pg_class where oid = 'stats.position'::regclass",
    )
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
            coalesce(u.display_name, a.display_name) as updated_by \
         from stats.event_capture ec left join identity.users u on u.id = ec.updated_by \
         left join access.actors a on a.id = ec.updated_by_actor \
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
    by: &Attribution,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into stats.event_capture (event_id, enabled, pre_minutes, post_minutes, updated_by, updated_by_actor)
         values ($1, $2, $3, $4, $5, $6)
         on conflict (event_id) do update set
             enabled = excluded.enabled,
             pre_minutes = excluded.pre_minutes,
             post_minutes = excluded.post_minutes,
             updated_by = excluded.updated_by, updated_by_actor = excluded.updated_by_actor",
    )
    .bind(event_id)
    .bind(enabled)
    .bind(pre_minutes)
    .bind(post_minutes)
    .bind(&by.user_id)
    .bind(&by.actor_id)
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

/// Arrivals, departures, and distinct pilots for each featured airport during the window.
///
/// **Movements are observed, not filed** (#433). They come from `stats.flight_leg`, which
/// `feed/delays.rs` writes one row into per detected wheels-up or touchdown, keyed on `end_time` —
/// the movement instant itself. Counting `stats.flight` instead meant a movement was only ever
/// *inferred*, from a filed plan plus a connection that overlapped the window, which counted a pilot
/// who logged on and never moved, an overflight at its filed destination, and a long-haul that was
/// merely connected — as both a departure and an arrival.
///
/// It also fixes reconnects for free, and that is the non-obvious part: a reconnect gets a new
/// `logon_time`, so `session_id` changes and `stats.flight` gains a second row for the same flight
/// (`migrations/0039_stats.sql:17-18`). One wheels-up is still one leg, so it is still one departure.
///
/// `unique_pilots` deliberately stays on `stats.flight`: it answers "who took part", which is not the
/// same question as "what moved", and a pilot who connected without flying still took part. A pilot
/// who both arrived at and departed from the same field is one unique pilot but two movements.
pub async fn event_airport_breakdown(
    pool: &PgPool,
    icaos: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<AirportBreakdown>, ApiError> {
    sqlx::query_as::<_, AirportBreakdown>(
        // `full join` rather than an inner one: an airport can have movements with no overlapping
        // connection row, or connections with no movement, and either way it belongs in the result.
        "select coalesce(m.icao, p.icao) as icao,
                coalesce(m.arrivals, 0) as arrivals,
                coalesce(m.departures, 0) as departures,
                coalesce(p.unique_pilots, 0) as unique_pilots
         from (
            select airport as icao,
                   count(*) filter (where kind = 'arrival') as arrivals,
                   count(*) filter (where kind = 'departure') as departures
              from stats.flight_leg
              where airport = any($3) and end_time >= $1 and end_time <= $2
              group by airport
         ) m
         full join (
            select icao, count(distinct cid) as unique_pilots from (
               select arrival as icao, cid from stats.flight
                 where arrival = any($3) and status <> 'prefiled'
                   and first_seen <= $2 and last_seen >= $1
               union all
               select departure as icao, cid from stats.flight
                 where departure = any($3) and status <> 'prefiled'
                   and first_seen <= $2 and last_seen >= $1
            ) t group by icao
         ) p on p.icao = m.icao
         order by (coalesce(m.arrivals, 0) + coalesce(m.departures, 0)) desc",
    )
    .bind(from)
    .bind(to)
    .bind(icaos)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Freeze an event's per-airport breakdown, so it survives leg retention (#433).
///
/// A **replace**, not just an upsert: the event's rows for airports absent from `rows` are deleted in
/// the same transaction. Upserting alone left a dropped airport behind with counts and a window from
/// an earlier freeze, and since [`event_movements_snapshot`] reads the reported window off the
/// busiest row, that stale row could both inflate `combined` and mislabel the entire response
/// (#433 review). The delete and the insert share one transaction so a concurrent read never sees a
/// half-replaced snapshot.
pub async fn snapshot_event_movements(
    pool: &PgPool,
    event_id: i64,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    rows: &[AirportBreakdown],
) -> Result<u64, ApiError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let icaos: Vec<String> = rows.iter().map(|r| r.icao.clone()).collect();
    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("delete from stats.event_movements where event_id = $1 and icao <> all($2)")
        .bind(event_id)
        .bind(&icaos)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        "insert into stats.event_movements \
         (event_id, icao, arrivals, departures, unique_pilots, window_start, window_end) ",
    );
    qb.push_values(rows, |mut b, r| {
        b.push_bind(event_id)
            .push_bind(&r.icao)
            .push_bind(r.arrivals)
            .push_bind(r.departures)
            .push_bind(r.unique_pilots)
            .push_bind(from)
            .push_bind(to);
    });
    qb.push(
        " on conflict (event_id, icao) do update set \
         arrivals = excluded.arrivals, departures = excluded.departures, \
         unique_pilots = excluded.unique_pilots, window_start = excluded.window_start, \
         window_end = excluded.window_end, captured_at = now()",
    );
    let res = qb
        .build()
        .persistent(false)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(res.rows_affected())
}

/// A frozen breakdown, with the window it was actually taken over.
pub struct MovementsSnapshot {
    pub rows: Vec<AirportBreakdown>,
    /// The window the counts cover. Reported instead of the event's current one: an event that was
    /// rescheduled after its capture closed still has these counts, and labelling them with the new
    /// times would describe them as something they are not (#433 review).
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct SnapshotRow {
    icao: String,
    arrivals: i64,
    departures: i64,
    unique_pilots: i64,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
}

/// An event's own window, for the movement-snapshot backfill.
#[derive(Debug, sqlx::FromRow)]
pub struct EventWindow {
    pub event_id: i64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

/// Events that finished with a saved capture but have no frozen movement breakdown (#433 review).
///
/// The close transition is the only thing that writes a snapshot, and it needs an *open* capture — so
/// every event that closed before `stats.event_movements` existed would never get one, keep computing
/// from `stats.flight_leg`, and drop to zero as its legs crossed `DELAY_LEG_RETAIN_DAYS`. This is
/// what lets the scheduler freeze them retroactively, while their legs are still there.
///
/// Excludes events with an *open* capture: one of those is being recorded again (rescheduled), and
/// its numbers are not final yet.
///
/// Bounded to the last `leg_retain_days`, which is what makes this cheap enough to run on every
/// scheduler tick. An event whose window ended before that has no legs left to count, so it can never
/// be backfilled — and because the all-zero guard deliberately declines to freeze it, an unbounded
/// query would re-answer and re-compute it once a minute forever.
pub async fn events_missing_movement_snapshot(
    pool: &PgPool,
    leg_retain_days: i64,
) -> Result<Vec<EventWindow>, ApiError> {
    sqlx::query_as::<_, EventWindow>(
        "select e.id as event_id, e.start_time, e.end_time \
         from stats.event_capture ec join events.event e on e.id = ec.event_id \
         where e.end_time < now() \
           and e.end_time > now() - make_interval(days => $1::int) \
           and exists (select 1 from stats.capture c \
                        where c.event_id = e.id and c.status = 'saved') \
           and not exists (select 1 from stats.capture c \
                            where c.event_id = e.id and c.status = 'open') \
           and not exists (select 1 from stats.event_movements m where m.event_id = e.id) \
         order by e.end_time desc",
    )
    .bind(leg_retain_days)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// The frozen breakdown for an event, or `None` when it was never snapshotted.
pub async fn event_movements_snapshot(
    pool: &PgPool,
    event_id: i64,
) -> Result<Option<MovementsSnapshot>, ApiError> {
    let rows = sqlx::query_as::<_, SnapshotRow>(
        "select icao, arrivals, departures, unique_pilots, window_start, window_end \
         from stats.event_movements \
         where event_id = $1 order by (arrivals + departures) desc, icao",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(db)?;

    let Some(first) = rows.first() else {
        return Ok(None);
    };
    // Safe because `snapshot_event_movements` replaces rather than upserts: every row for an event
    // comes from the same freeze, so any of them carries that freeze's window. The `, icao`
    // tiebreaker above makes which one deterministic when two airports tie (#433 review).
    let (window_start, window_end) = (first.window_start, first.window_end);
    Ok(Some(MovementsSnapshot {
        window_start,
        window_end,
        rows: rows
            .into_iter()
            .map(|r| AirportBreakdown {
                icao: r.icao,
                arrivals: r.arrivals,
                departures: r.departures,
                unique_pilots: r.unique_pilots,
            })
            .collect(),
    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // ---- #689: batch inserts don't grow the statement cache ----------------------------------------

    /// Every stats batch writer, once, with `n` rows each — the shape a collector tick and the delay /
    /// taxi / event jobs write in.
    async fn write_every_batch(pool: &PgPool, n: usize, event_id: i64) {
        let now = Utc::now();
        let ids = 0..n as i64;
        let mut tx = pool.begin().await.unwrap();
        let members: Vec<(i32, String)> = ids
            .clone()
            .map(|i| (900_000 + i as i32, format!("M{i}")))
            .collect();
        upsert_members(&mut tx, &members, now).await.unwrap();
        let flights: Vec<FlightRow> = ids
            .clone()
            .map(|i| FlightRow {
                session_id: 7_000 + i,
                cid: 900_000 + i as i32,
                callsign: format!("TST{i}"),
                server: None,
                logon_time: now,
                flight_rules: Some("I".into()),
                departure: Some("KJFK".into()),
                arrival: Some("KBOS".into()),
                alternate: None,
                aircraft_short: Some("B738".into()),
                aircraft_faa: None,
                cruise_tas: None,
                cruise_alt: None,
                deptime: None,
                enroute_time: None,
                route: None,
                remarks: None,
                revision_id: Some(n as i32),
            })
            .collect();
        upsert_flights(&mut tx, &flights, now).await.unwrap();
        insert_flight_plan_revisions(&mut tx, &flights, now)
            .await
            .unwrap();
        let positions: Vec<PositionRow> = ids
            .clone()
            .map(|i| PositionRow {
                session_id: 7_000 + i,
                lat: 40.0,
                lon: -73.0,
                altitude: 1000,
                groundspeed: 200,
                heading: 90,
                transponder: None,
                qnh_mb: None,
            })
            .collect();
        insert_positions(&mut tx, &positions, now).await.unwrap();
        let prefiles: Vec<PrefileRow> = ids
            .clone()
            .map(|i| PrefileRow {
                session_id: 8_000 + i,
                cid: 900_000 + i as i32,
                callsign: format!("PRE{i}"),
                departure: None,
                arrival: None,
                alternate: None,
                aircraft_short: None,
                route: None,
                remarks: None,
                cruise_alt: None,
                revision_id: None,
            })
            .collect();
        upsert_prefiles(&mut tx, &prefiles, now).await.unwrap();
        let controllers: Vec<ControllerRow> = ids
            .clone()
            .map(|i| ControllerRow {
                session_id: 9_000 + i,
                cid: 900_000 + i as i32,
                callsign: format!("ZNY_{i}_CTR"),
                frequency: None,
                facility: None,
                rating: None,
                server: None,
                visual_range: None,
                atis_code: None,
                logon_time: now,
                is_atis: false,
            })
            .collect();
        upsert_controllers(&mut tx, &controllers, now)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let legs: Vec<FlightLegRow> = ids
            .clone()
            .map(|i| FlightLegRow {
                kind: "departure",
                airport: "KJFK".into(),
                callsign: format!("TST{i}"),
                cid: 900_000 + i as i32,
                aircraft: None,
                runway: None,
                procedure: None,
                start: now,
                end: now,
                duration_sec: 60,
            })
            .collect();
        insert_flight_legs(pool, &legs).await.unwrap();
        let taxi: Vec<TaxiObservationRow> = ids
            .clone()
            .map(|_| TaxiObservationRow {
                airport: "KJFK".into(),
                gate_id: None,
                aircraft: None,
                runway: None,
                pushback_sec: None,
                startup_sec: None,
                taxi_sec: 300,
                observed_at: now,
            })
            .collect();
        insert_taxi_observations(pool, &taxi).await.unwrap();
        let movements: Vec<AirportBreakdown> = ids
            .map(|i| AirportBreakdown {
                icao: format!("K{i:03}"),
                arrivals: 1,
                departures: 1,
                unique_pilots: 1,
            })
            .collect();
        snapshot_event_movements(pool, event_id, now, now, &movements)
            .await
            .unwrap();
    }

    /// The leak in #689: a batch insert's SQL text depends on its row count, so a cached prepared
    /// statement is a new cache entry every tick. Through a one-connection pool, so every statement
    /// lands on the connection whose cache is read — and with *varying* row counts, since a fixed
    /// count would pass even with the leak.
    #[sqlx::test]
    async fn batch_inserts_do_not_grow_the_statement_cache(pool: PgPool) {
        use sqlx::Connection as _;
        let event_id = 68_900_i64;
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) values ($1, 'Test', now(), now())",
        )
        .bind(event_id)
        .execute(&pool)
        .await
        .unwrap();
        let one = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();

        // Warm up: the fixed-SQL statements around the batches are cached once, legitimately.
        write_every_batch(&one, 1, event_id).await;
        let cached =
            |pool: PgPool| async move { pool.acquire().await.unwrap().cached_statements_size() };
        let baseline = cached(one.clone()).await;

        for n in [2, 3, 5, 8] {
            write_every_batch(&one, n, event_id).await;
        }
        assert_eq!(
            cached(one.clone()).await,
            baseline,
            "a batch insert kept a prepared statement per row count"
        );
    }

    /// A movement is a detected wheels-up or touchdown, not a filed plan (#433). These fixtures are
    /// the four shapes that used to be counted and should not be, plus the one that should.
    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, hour, minute, 0).unwrap()
    }

    /// The event window every test counts over: 12:00–14:00.
    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        (at(12, 0), at(14, 0))
    }

    /// A connection: someone logged on with a filed plan. Says nothing about whether they moved.
    async fn connection(
        pool: &PgPool,
        session_id: i64,
        cid: i32,
        departure: &str,
        arrival: &str,
        first_seen: DateTime<Utc>,
        last_seen: DateTime<Utc>,
    ) {
        sqlx::query(
            "insert into stats.flight \
             (session_id, cid, callsign, logon_time, first_seen, last_seen, status, departure, arrival) \
             values ($1, $2, $3, $4, $4, $5, 'active', $6, $7)",
        )
        .bind(session_id)
        .bind(cid)
        .bind(format!("TEST{session_id}"))
        .bind(first_seen)
        .bind(last_seen)
        .bind(departure)
        .bind(arrival)
        .execute(pool)
        .await
        .unwrap();
    }

    /// An observed movement: wheels-up or touchdown at `end_time`.
    async fn movement(pool: &PgPool, kind: &str, airport: &str, cid: i32, end: DateTime<Utc>) {
        sqlx::query(
            "insert into stats.flight_leg \
             (kind, airport, callsign, cid, start_time, end_time, duration_sec) \
             values ($1, $2, $3, $4, $5, $6, 600)",
        )
        .bind(kind)
        .bind(airport)
        .bind(format!("TEST{cid}"))
        .bind(cid)
        .bind(end - chrono::Duration::minutes(10))
        .bind(end)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn counts(pool: &PgPool, icaos: &[&str]) -> Vec<AirportBreakdown> {
        let (from, to) = window();
        let owned: Vec<String> = icaos.iter().map(|s| s.to_string()).collect();
        event_airport_breakdown(pool, &owned, from, to)
            .await
            .unwrap()
    }

    fn find<'a>(rows: &'a [AirportBreakdown], icao: &str) -> Option<&'a AirportBreakdown> {
        rows.iter().find(|r| r.icao == icao)
    }

    /// AC2. The biggest single source of inflation: a pilot who connects, files, and disconnects at
    /// the gate used to be a full departure.
    #[sqlx::test]
    async fn a_connection_that_never_moved_is_not_a_movement(pool: PgPool) {
        connection(&pool, 1, 1001, "KJFK", "KBOS", at(12, 10), at(12, 40)).await;

        let rows = counts(&pool, &["KJFK", "KBOS"]).await;

        let jfk = find(&rows, "KJFK").expect("KJFK present — they were connected there");
        assert_eq!(jfk.departures, 0, "never rolled, so never departed");
        assert_eq!(jfk.arrivals, 0);
        // They still took part, which is a different question from whether they moved.
        assert_eq!(jfk.unique_pilots, 1);
    }

    /// AC3. An overflight, a diversion or a crash used to count at the filed destination.
    #[sqlx::test]
    async fn a_flight_that_never_lands_is_not_an_arrival(pool: PgPool) {
        connection(&pool, 2, 1002, "KJFK", "KBOS", at(12, 0), at(13, 30)).await;
        movement(&pool, "departure", "KJFK", 1002, at(12, 20)).await;

        let rows = counts(&pool, &["KJFK", "KBOS"]).await;

        assert_eq!(
            find(&rows, "KJFK").unwrap().departures,
            1,
            "it did take off"
        );
        let bos = find(&rows, "KBOS").expect("KBOS present — a plan was filed to it");
        assert_eq!(bos.arrivals, 0, "it never touched down");
    }

    /// AC5. `session_id = fnv1a(cid, logon_time)`, so a reconnect mints a second `stats.flight` row
    /// for one flight (migrations/0039_stats.sql:17-18) — and used to mint a second departure.
    #[sqlx::test]
    async fn a_reconnect_during_one_flight_is_one_departure(pool: PgPool) {
        connection(&pool, 3, 1003, "KJFK", "KBOS", at(12, 0), at(12, 30)).await;
        connection(&pool, 4, 1003, "KJFK", "KBOS", at(12, 31), at(13, 30)).await;
        // One aircraft, one wheels-up, however many times its pilot dropped.
        movement(&pool, "departure", "KJFK", 1003, at(12, 15)).await;

        let rows = counts(&pool, &["KJFK"]).await;

        let jfk = find(&rows, "KJFK").unwrap();
        assert_eq!(jfk.departures, 1, "two sessions, one flight, one departure");
        assert_eq!(jfk.unique_pilots, 1, "and one pilot");
    }

    /// AC4. The window is the event's own, and a movement outside it belongs to another event — the
    /// old predicate counted any connection merely *overlapping* the window, so a long-haul that
    /// pushed hours earlier landed in the totals.
    #[sqlx::test]
    async fn a_movement_outside_the_event_window_is_not_counted(pool: PgPool) {
        movement(&pool, "departure", "KJFK", 1004, at(11, 30)).await; // before
        movement(&pool, "departure", "KJFK", 1005, at(13, 0)).await; // inside
        movement(&pool, "arrival", "KJFK", 1006, at(14, 30)).await; // after

        let rows = counts(&pool, &["KJFK"]).await;

        let jfk = find(&rows, "KJFK").unwrap();
        assert_eq!(jfk.departures, 1);
        assert_eq!(jfk.arrivals, 0);
    }

    /// Seed an event, since `stats.event_movements` is keyed to one by foreign key.
    async fn event(pool: &PgPool, id: i64) {
        let (from, to) = window();
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) values ($1, 'Test', $2, $3)",
        )
        .bind(id)
        .bind(from)
        .bind(to)
        .execute(pool)
        .await
        .unwrap();
    }

    fn breakdown(icao: &str, arrivals: i64, departures: i64) -> AirportBreakdown {
        AirportBreakdown {
            icao: icao.to_string(),
            arrivals,
            departures,
            unique_pilots: 3,
        }
    }

    /// AC7. Legs are pruned at `DELAY_LEG_RETAIN_DAYS`, so an event recomputed from them reports zero
    /// once they age out — correct numbers that quietly disappear. The snapshot is what stops that,
    /// and none of it was covered: blanking either half left the whole suite green (#433 review).
    #[sqlx::test]
    async fn a_frozen_breakdown_reads_back_with_the_window_it_was_taken_over(pool: PgPool) {
        let (from, to) = window();
        event(&pool, 900).await;

        let written = snapshot_event_movements(
            &pool,
            900,
            from,
            to,
            &[breakdown("KJFK", 4, 6), breakdown("KBOS", 1, 2)],
        )
        .await
        .unwrap();
        assert_eq!(written, 2);

        let snap = event_movements_snapshot(&pool, 900)
            .await
            .unwrap()
            .expect("a snapshot was just written");

        // Busiest first, as the live query orders.
        assert_eq!(snap.rows[0].icao, "KJFK");
        assert_eq!((snap.rows[0].arrivals, snap.rows[0].departures), (4, 6));
        assert_eq!((snap.rows[1].arrivals, snap.rows[1].departures), (1, 2));
        // The window travels with the counts, so a rescheduled event cannot mislabel them.
        assert_eq!((snap.window_start, snap.window_end), (from, to));
    }

    /// The scheduler pass is idempotent, so a re-close must overwrite rather than duplicate — the
    /// primary key would reject the second insert outright without `on conflict`.
    #[sqlx::test]
    async fn re_freezing_an_event_replaces_its_counts_rather_than_duplicating_them(pool: PgPool) {
        let (from, to) = window();
        event(&pool, 901).await;

        snapshot_event_movements(&pool, 901, from, to, &[breakdown("KJFK", 1, 1)])
            .await
            .unwrap();
        snapshot_event_movements(&pool, 901, from, to, &[breakdown("KJFK", 9, 9)])
            .await
            .unwrap();

        let snap = event_movements_snapshot(&pool, 901).await.unwrap().unwrap();
        assert_eq!(snap.rows.len(), 1, "one row per (event, airport)");
        assert_eq!((snap.rows[0].arrivals, snap.rows[0].departures), (9, 9));
    }

    /// A re-freeze is a **replace**. Upserting alone left a dropped airport behind with counts and a
    /// window from the earlier freeze, and since the window is read off the busiest row, that stale
    /// row both inflated `combined` and mislabelled the whole response. Proven against the real
    /// schema before the fix: KBOS(50/50)@12:00–14:00 survived beside KJFK(2/2)@18:00–20:00 and, being
    /// busiest, supplied 12:00–14:00 as the reported window for a 4-movement event reporting 104
    /// (#433 review).
    #[sqlx::test]
    async fn re_freezing_with_fewer_airports_drops_the_ones_no_longer_featured(pool: PgPool) {
        let (from, to) = window();
        event(&pool, 903).await;

        snapshot_event_movements(
            &pool,
            903,
            from,
            to,
            &[breakdown("KBOS", 50, 50), breakdown("KJFK", 1, 1)],
        )
        .await
        .unwrap();

        // Rescheduled, and KBOS is no longer a featured airport: a second freeze over a later window.
        let (from2, to2) = (at(18, 0), at(20, 0));
        snapshot_event_movements(&pool, 903, from2, to2, &[breakdown("KJFK", 2, 2)])
            .await
            .unwrap();

        let snap = event_movements_snapshot(&pool, 903).await.unwrap().unwrap();
        assert_eq!(snap.rows.len(), 1, "KBOS is gone, not left behind at 50/50");
        assert_eq!(snap.rows[0].icao, "KJFK");
        assert_eq!((snap.rows[0].arrivals, snap.rows[0].departures), (2, 2));
        assert_eq!(
            (snap.window_start, snap.window_end),
            (from2, to2),
            "the window of the freeze that is actually in the table"
        );
    }

    /// An event that was never snapshotted must say so, not return an empty breakdown — the read
    /// path tells "frozen, and it was zero" from "not frozen, compute it" by exactly this.
    #[sqlx::test]
    async fn an_event_that_was_never_frozen_has_no_snapshot(pool: PgPool) {
        event(&pool, 902).await;

        assert!(
            event_movements_snapshot(&pool, 902)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// A real turnaround still counts twice, which is the behaviour the docs promise.
    #[sqlx::test]
    async fn a_turnaround_is_one_pilot_and_two_movements(pool: PgPool) {
        connection(&pool, 5, 1007, "KJFK", "KJFK", at(12, 0), at(13, 45)).await;
        movement(&pool, "arrival", "KJFK", 1007, at(12, 30)).await;
        movement(&pool, "departure", "KJFK", 1007, at(13, 30)).await;

        let rows = counts(&pool, &["KJFK"]).await;

        let jfk = find(&rows, "KJFK").unwrap();
        assert_eq!(jfk.arrivals + jfk.departures, 2);
        assert_eq!(jfk.unique_pilots, 1);
    }
}

#[cfg(test)]
mod capture_release_tests {
    use chrono::TimeZone;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, hour, 0, 0).unwrap()
    }

    /// Ten positions on one session across the hour the capture will cover.
    async fn positions(pool: &PgPool) {
        for i in 0..10i32 {
            sqlx::query(
                "insert into stats.position \
                 (session_id, ts, lat, lon, altitude, groundspeed, heading) \
                 values (1, $1, 0, 0, 0, 0, 0)",
            )
            .bind(at(12) + chrono::Duration::minutes(i as i64))
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn saved_capture(pool: &PgPool) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into stats.capture (label, start_time, end_time, status) \
             values ('w', $1, $2, 'saved') returning id",
        )
        .bind(at(12))
        .bind(at(13))
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn remaining(pool: &PgPool) -> i64 {
        sqlx::query_scalar::<_, i64>("select count(*) from stats.position")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// AC2 — the whole point of #432. A saved capture pins its positions against compaction; the
    /// delete is only worth anything if discarding it lets them go. That step was asserted by a
    /// comment and by nothing else: `downsample_positions` had no test in the repo at all
    /// (#432 review).
    #[sqlx::test]
    async fn discarding_a_capture_releases_the_positions_it_was_pinning(pool: PgPool) {
        positions(&pool).await;
        let id = saved_capture(&pool).await;

        // Saved: CAPTURE_GUARD protects every row in the window, so compaction takes nothing.
        let thinned = downsample_positions(&pool, at(11), at(14), 2)
            .await
            .unwrap();
        assert_eq!(thinned, 0, "a saved capture must pin its positions");
        assert_eq!(remaining(&pool).await, 10);

        assert!(discard_capture(&pool, &id).await.unwrap());

        // Discarded: out of the guard, so the same pass now thins them.
        let thinned = downsample_positions(&pool, at(11), at(14), 2)
            .await
            .unwrap();
        assert!(
            thinned > 0,
            "discarding must let compaction reclaim the space"
        );
        assert_eq!(
            remaining(&pool).await,
            5,
            "keep_every = 2 keeps every second row"
        );
    }

    /// The guard is on `status in ('open','saved')`, so an *open* capture pins too — otherwise a
    /// recording in progress would be thinned underneath itself.
    #[sqlx::test]
    async fn an_open_capture_pins_its_positions_as_well(pool: PgPool) {
        positions(&pool).await;
        sqlx::query(
            "insert into stats.capture (label, start_time, end_time, status) \
             values ('w', $1, $2, 'open')",
        )
        .bind(at(12))
        .bind(at(13))
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            downsample_positions(&pool, at(11), at(14), 2)
                .await
                .unwrap(),
            0
        );
    }

    /// #432 review: an event capture that is still open and still inside the scheduler's padded
    /// window must not be discardable — `capture_scheduler_once` would open a replacement on its
    /// next pass and the positions would stay pinned, after the delete reported success.
    #[sqlx::test]
    async fn an_event_capture_still_recording_is_reported_live(pool: PgPool) {
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) \
             values (500, 'E', now() - interval '10 minutes', now() + interval '1 hour')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("insert into stats.event_capture (event_id, enabled) values (500, true)")
            .execute(&pool)
            .await
            .unwrap();
        let open = sqlx::query_scalar::<_, String>(
            "insert into stats.capture (event_id, label, start_time, status) \
             values (500, 'E', now() - interval '40 minutes', 'open') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(event_capture_is_live(&pool, &open).await.unwrap());

        // Once it is saved, the scheduler no longer reopens and it is deletable.
        sqlx::query("update stats.capture set status = 'saved', end_time = now() where id = $1")
            .bind(&open)
            .execute(&pool)
            .await
            .unwrap();
        assert!(!event_capture_is_live(&pool, &open).await.unwrap());
    }

    /// The refusal is scoped to the window, not to "event capture that is open". Past its window the
    /// scheduler closes a capture rather than reopening it, so refusing there would strand a stale
    /// open row as permanently undeletable — the exact unreclaimable storage #432 exists to fix.
    /// (Caught by mutation: dropping the window clause left every other case green.)
    #[sqlx::test]
    async fn an_open_event_capture_past_its_window_is_not_live(pool: PgPool) {
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) \
             values (502, 'E', now() - interval '6 hours', now() - interval '5 hours')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("insert into stats.event_capture (event_id, enabled) values (502, true)")
            .execute(&pool)
            .await
            .unwrap();
        let stale = sqlx::query_scalar::<_, String>(
            "insert into stats.capture (event_id, label, start_time, status) \
             values (502, 'E', now() - interval '7 hours', 'open') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(
            !event_capture_is_live(&pool, &stale).await.unwrap(),
            "the scheduler will not reopen past the window, so this must stay deletable"
        );
        assert!(discard_capture(&pool, &stale).await.unwrap());
    }

    /// Likewise when the event's capture is switched off: nothing will reopen it.
    #[sqlx::test]
    async fn an_open_event_capture_with_capturing_disabled_is_not_live(pool: PgPool) {
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) \
             values (503, 'E', now() - interval '10 minutes', now() + interval '1 hour')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("insert into stats.event_capture (event_id, enabled) values (503, false)")
            .execute(&pool)
            .await
            .unwrap();
        let id = sqlx::query_scalar::<_, String>(
            "insert into stats.capture (event_id, label, start_time, status) \
             values (503, 'E', now() - interval '40 minutes', 'open') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(!event_capture_is_live(&pool, &id).await.unwrap());
    }

    /// An ad-hoc open capture has no scheduler behind it, so it stays discardable.
    #[sqlx::test]
    async fn an_ad_hoc_open_capture_is_not_live(pool: PgPool) {
        let id = sqlx::query_scalar::<_, String>(
            "insert into stats.capture (label, start_time, status) \
             values ('adhoc', now(), 'open') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(!event_capture_is_live(&pool, &id).await.unwrap());
        assert!(discard_capture(&pool, &id).await.unwrap());
    }
}

/// #776: the ten-minute taxi-sample reload must not OOM the backend.
#[cfg(test)]
mod taxi_reload_tests {
    use super::*;

    /// `observations` rows over `airports` airports, inserted round-robin so the table's physical order
    /// is not airport order. One in five of each airport's rows has a runway; none has a gate (that
    /// column is an FK).
    async fn seed_taxi_observations(pool: &PgPool, airports: i64, observations: i64) {
        sqlx::query(
            "insert into stats.taxi_observation \
               (airport, aircraft, runway, pushback_sec, startup_sec, taxi_sec, observed_at) \
             select 'T' || lpad((i % $1)::text, 3, '0'), 'B738', \
                    case when (i / $1) % 5 = 0 then '27L' end, 120, 60, 300 + (i % 7), now() \
             from generate_series(0, $2 - 1) i",
        )
        .bind(airports)
        .bind(observations)
        .execute(pool)
        .await
        .unwrap();
    }

    /// The fewest bytes the returned map can occupy: every sample and string at exactly its size, and
    /// nothing for the hash table itself.
    fn least_bytes(
        map: &std::collections::HashMap<String, Vec<crate::feed::taxi_estimate::TaxiSample>>,
    ) -> usize {
        let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        map.iter()
            .map(|(airport, samples)| {
                airport.len()
                    + samples.len() * std::mem::size_of::<crate::feed::taxi_estimate::TaxiSample>()
                    + samples
                        .iter()
                        .map(|s| text(&s.gate_id) + text(&s.aircraft) + text(&s.runway))
                        .sum::<usize>()
            })
            .sum()
    }

    /// #776: the refresh job reloads every taxi observation every ten minutes while the previous map is
    /// still live. On production's 352k rows the old loader allocated ~108 MB to build a ~48 MB map,
    /// which OOM-killed each 512 MiB replica ten minutes after it started and dropped every realtime
    /// socket on it. A reload may hold the map it builds and little else, and the map must be trimmed.
    ///
    /// Sizes are measured against the map actually returned, not derived from the loader: 65 samples
    /// per airport is the worst case for a doubling `Vec` (capacity 128), so an untrimmed map or a
    /// second copy of the rows is far outside the bounds below.
    #[sqlx::test]
    async fn reloading_taxi_samples_holds_one_trimmed_map(pool: PgPool) {
        seed_taxi_observations(&pool, 1_000, 65_000).await;
        // The map the job already holds when it reloads (and a warm connection and statement cache).
        let previous = load_all_taxi_samples(&pool).await.unwrap();

        let (reloaded, peak, held) =
            crate::alloc_probe::measure(load_all_taxi_samples(&pool)).await;
        let reloaded = reloaded.unwrap();

        assert_eq!(reloaded.len(), 1_000, "every airport");
        assert!(
            reloaded.values().all(|samples| samples.len() == 65),
            "every airport's samples, and only its own"
        );
        let one = &reloaded["T007"];
        assert_eq!(
            one.iter()
                .filter(|s| s.runway.as_deref() == Some("27L"))
                .count(),
            13
        );
        assert_eq!(
            one.iter()
                .map(|s| s.taxi_sec)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            7
        );

        let least = least_bytes(&reloaded);
        assert!(
            held * 10 <= least * 11,
            "the map is trimmed: holds {held} bytes for {least} bytes of samples"
        );
        assert!(
            peak * 4 <= least * 5,
            "a reload peaks at the map it builds: {peak} bytes for {least} bytes of samples"
        );
        drop(previous);
    }
}
