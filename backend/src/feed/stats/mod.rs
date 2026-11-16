//! Persistent stats collection. A background worker reads OIS's existing in-memory feed snapshot
//! each cycle (no second HTTP poll), decomposes it into flight/controller sessions + a position
//! time-series + a network-totals row, and closes sessions that disappear (computing a summary +
//! Douglas–Peucker track). Ambient collection is scoped to US/VATUSA-relevant traffic; while a
//! `stats.capture` window is open the scope is relaxed so events are captured in full.
//!
//! Ported from the standalone `stats` ingester (`crates/ingester`), adapted to reuse OIS's feed and
//! run against plain Postgres. See `~/Programing/stats/DESIGN.md`.

mod geo;
pub mod reconstruct;
mod session;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::feed::FeedState;
use crate::feed::airports::{AirportDb, IataMap};
use crate::feed::airspace::Boundaries;
use crate::feed::vatsim::{Atis, Controller, FlightPlan, Pilot, Prefile};
use crate::repos::stats as repo;

use geo::TrackPoint;
use session::{parse_filed_altitude, parse_ts, session_id};

/// How often the collector wakes. Slightly under the 15s feed cadence so it never misses a new
/// snapshot; duplicate reads are dropped by the `source_timestamp` dedupe.
const COLLECT_SECS: u64 = 10;
/// A session absent this long (and not in the current tick) is considered closed.
const GRACE: chrono::Duration = chrono::Duration::seconds(180);
/// Douglas–Peucker tolerance in degrees (~0.6 nm) for the on-close simplified track.
const DP_EPSILON: f64 = 0.01;

/// US ICAO prefixes (CONUS + Alaska/Hawaii/Pacific + Caribbean territories).
const US_ICAO_PREFIXES: &[&str] = &["K", "PA", "PH", "PG", "PW", "PM", "TJ", "TI"];

/// In-memory record of a live session so we can dedupe positions and close it on disappearance.
struct Active {
    last_seen: DateTime<Utc>,
    last_updated: String,
    revision_id: Option<i32>,
}

/// Spawn the stats collector (DB-gated). Reuses OIS's single feed poll via the shared snapshot.
pub fn spawn_collector(pool: PgPool, feed: FeedState, airspace: Arc<Boundaries>) {
    tokio::spawn(async move { collector(pool, feed, airspace).await });
}

async fn collector(pool: PgPool, feed: FeedState, airspace: Arc<Boundaries>) {
    let mut pilots: HashMap<i64, Active> = HashMap::new();
    let mut controllers: HashMap<i64, Active> = HashMap::new();
    match repo::list_active_flights(&pool).await {
        Ok(rows) => {
            for (sid, last_seen, rev) in rows {
                pilots.insert(
                    sid,
                    Active {
                        last_seen,
                        last_updated: String::new(),
                        revision_id: rev,
                    },
                );
            }
            tracing::info!(reloaded = pilots.len(), "stats: restored active flights");
        }
        Err(e) => tracing::warn!(error = %e, "stats: could not reload active flights"),
    }

    let mut last_source = String::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(COLLECT_SECS));
    loop {
        ticker.tick().await;
        match tick(
            &pool,
            &feed,
            &airspace,
            &mut pilots,
            &mut controllers,
            &last_source,
        )
        .await
        {
            Ok(Some(src)) => last_source = src,
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "stats: collection tick failed"),
        }
    }
}

/// Process one snapshot. Returns the new `source_timestamp`, or `None` when there's nothing to do
/// (no snapshot yet, or the feed re-served the same one).
async fn tick(
    pool: &PgPool,
    feed: &FeedState,
    airspace: &Boundaries,
    pilots: &mut HashMap<i64, Active>,
    controllers: &mut HashMap<i64, Active>,
    last_source: &str,
) -> Result<Option<String>, ApiError> {
    // Grab the shared snapshot + reference maps, then drop the feed lock before any DB work.
    let (snap, iata, airports) = {
        let guard = feed.read().await;
        match guard.snapshot.clone() {
            Some(s) => (s, guard.iata.clone(), guard.airports.clone()),
            None => return Ok(None),
        }
    };
    if snap.source_timestamp.is_empty() || snap.source_timestamp == last_source {
        return Ok(None);
    }
    let now = parse_ts(&snap.source_timestamp).unwrap_or(snap.fetched_at);
    let data = &snap.data;

    // While a capture is open, store everything present (relax the US scope filter).
    let relax = repo::has_open_capture(pool).await.unwrap_or(false);

    let mut member_names: HashMap<i32, String> = HashMap::new();
    let mut seen_pilots: HashSet<i64> = HashSet::new();
    let mut flight_rows: Vec<repo::FlightRow> = Vec::new();
    let mut position_rows: Vec<repo::PositionRow> = Vec::new();

    for p in &data.pilots {
        if p.cid == 0 || p.logon_time.is_empty() {
            continue; // can't derive a stable session key
        }
        if !relax && !is_us_relevant_pilot(p, airspace) {
            continue;
        }
        let sid = session_id(p.cid, &p.logon_time);
        seen_pilots.insert(sid);
        member_names.insert(p.cid, p.name.clone());
        let rev = p.flight_plan.as_ref().and_then(|fp| fp.revision_id);

        let want_pos = match pilots.get_mut(&sid) {
            Some(a) => {
                let changed = a.last_updated != p.last_updated;
                if changed {
                    a.last_updated = p.last_updated.clone();
                }
                a.last_seen = now;
                a.revision_id = rev;
                changed
            }
            None => {
                pilots.insert(
                    sid,
                    Active {
                        last_seen: now,
                        last_updated: p.last_updated.clone(),
                        revision_id: rev,
                    },
                );
                true
            }
        };
        flight_rows.push(flight_row(sid, p));
        if want_pos {
            position_rows.push(position_row(sid, p));
        }
    }

    // Prefiles: provisional flights (cid+callsign key), US-scoped by their filed dep/arr.
    let mut seen_prefiles: HashSet<i64> = HashSet::new();
    let mut prefile_rows: Vec<repo::PrefileRow> = Vec::new();
    for pf in &data.prefiles {
        if pf.cid == 0 {
            continue;
        }
        let us = pf
            .flight_plan
            .as_ref()
            .is_some_and(|fp| is_us_airport(&fp.departure) || is_us_airport(&fp.arrival));
        if !relax && !us {
            continue;
        }
        let sid = session_id(pf.cid, &format!("prefile:{}", pf.callsign));
        if seen_prefiles.insert(sid) {
            prefile_rows.push(prefile_row(sid, pf));
            member_names.insert(pf.cid, pf.name.clone());
        }
    }

    // Controllers + ATIS.
    let mut seen_controllers: HashSet<i64> = HashSet::new();
    let mut controller_rows: Vec<repo::ControllerRow> = Vec::new();
    for c in &data.controllers {
        if c.cid == 0 || c.logon_time.is_empty() {
            continue;
        }
        // Observers (facility 0) aren't providing ATC — they're just watching (often pilots in
        // observer mode). Don't record them as controller sessions.
        if c.facility == 0 {
            continue;
        }
        if !relax && !is_us_controller(&c.callsign, &iata, &airports) {
            continue;
        }
        let sid = session_id(c.cid, &c.logon_time);
        member_names.insert(c.cid, c.name.clone());
        note_controller(controllers, sid, now);
        if seen_controllers.insert(sid) {
            let is_atis = c.callsign.ends_with("_ATIS");
            controller_rows.push(controller_row_from_ctrl(sid, c, is_atis));
        }
    }
    for a in &data.atis {
        if a.cid == 0 || a.logon_time.is_empty() {
            continue;
        }
        if !relax && !is_us_controller(&a.callsign, &iata, &airports) {
            continue;
        }
        let sid = session_id(a.cid, &a.logon_time);
        member_names.insert(a.cid, a.name.clone());
        note_controller(controllers, sid, now);
        if seen_controllers.insert(sid) {
            controller_rows.push(controller_row_from_atis(sid, a));
        }
    }

    let members: Vec<(i32, String)> = member_names.into_iter().collect();
    let counts = repo::SnapshotCounts {
        connected_clients: data.general.connected_clients,
        unique_users: data.general.unique_users,
        pilots: seen_pilots.len() as i32,
        // Actual controllers only — exclude observers (facility 0).
        controllers: data.controllers.iter().filter(|c| c.facility != 0).count() as i32,
        atis: data.atis.len() as i32,
        prefiles: seen_prefiles.len() as i32,
    };

    // One transaction per tick.
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    repo::upsert_flights(&mut tx, &flight_rows, now).await?;
    // Record a plan revision for any flight whose plan changed this tick (for temporally-faithful
    // replay). Independent of the stats.flight overwrite above; unchanged flights insert nothing.
    repo::insert_flight_plan_revisions(&mut tx, &flight_rows, now).await?;
    if !position_rows.is_empty() {
        repo::insert_positions(&mut tx, &position_rows, now).await?;
    }
    repo::upsert_prefiles(&mut tx, &prefile_rows, now).await?;
    repo::upsert_controllers(&mut tx, &controller_rows, now).await?;
    repo::upsert_members(&mut tx, &members, now).await?;
    repo::insert_snapshot(&mut tx, now, &counts).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    // Close sessions that disappeared (outside the write tx).
    let cutoff = now - GRACE;
    let gone_pilots: Vec<i64> = pilots
        .iter()
        .filter(|(sid, a)| a.last_seen < cutoff && !seen_pilots.contains(sid))
        .map(|(sid, _)| *sid)
        .collect();
    // A session whose close fails stays in its map, so the next tick retries the close.
    for sid in gone_pilots {
        match close_flight(pool, sid).await {
            Ok(()) => {
                pilots.remove(&sid);
            }
            Err(e) => {
                tracing::warn!(sid, error = %e, "stats: failed to close flight; retrying next tick")
            }
        }
    }

    let gone_controllers: Vec<i64> = controllers
        .iter()
        .filter(|(sid, a)| a.last_seen < cutoff && !seen_controllers.contains(sid))
        .map(|(sid, _)| *sid)
        .collect();
    for sid in gone_controllers {
        match repo::close_controller(pool, sid).await {
            Ok(()) => {
                controllers.remove(&sid);
            }
            Err(e) => tracing::warn!(
                sid,
                error = %e,
                "stats: failed to close controller session; retrying next tick"
            ),
        }
    }

    tracing::debug!(
        pilots = seen_pilots.len(),
        controllers = seen_controllers.len(),
        positions = position_rows.len(),
        relax,
        "stats tick"
    );
    Ok(Some(snap.source_timestamp.clone()))
}

fn note_controller(map: &mut HashMap<i64, Active>, sid: i64, now: DateTime<Utc>) {
    map.entry(sid)
        .and_modify(|a| a.last_seen = now)
        .or_insert(Active {
            last_seen: now,
            last_updated: String::new(),
            revision_id: None,
        });
}

/// Read a closed flight's track, compute a summary + simplified path, and mark it completed.
async fn close_flight(pool: &PgPool, sid: i64) -> Result<(), ApiError> {
    let rows = repo::fetch_flight_track(pool, sid).await?;
    if rows.is_empty() {
        return repo::set_flight_completed(pool, sid, None, None, None, None, None).await;
    }
    let points: Vec<TrackPoint> = rows
        .iter()
        .map(|(ts, lat, lon, alt, _)| TrackPoint {
            ts: ts.timestamp(),
            lat: *lat as f64,
            lon: *lon as f64,
            alt: *alt,
        })
        .collect();
    let speeds: Vec<i32> = rows.iter().map(|r| r.4 as i32).collect();
    let s = geo::summarize(&points, &speeds, DP_EPSILON);
    repo::set_flight_completed(
        pool,
        sid,
        Some(s.duration_s),
        Some(s.distance_nm),
        Some(s.max_altitude),
        Some(s.max_groundspeed),
        Some(&s.path_simplified),
    )
    .await
}

// --- US-relevance ------------------------------------------------------------------------------

/// A 4-letter US-airspace ICAO (CONUS `K…` + territories).
fn is_us_airport(icao: &str) -> bool {
    let s = icao.trim().to_ascii_uppercase();
    s.len() == 4 && US_ICAO_PREFIXES.iter().any(|p| s.starts_with(p))
}

/// A pilot is US-relevant if it files to/from a US airport, or is physically over a US ARTCC.
fn is_us_relevant_pilot(p: &Pilot, airspace: &Boundaries) -> bool {
    let filed_us = p
        .flight_plan
        .as_ref()
        .is_some_and(|fp| is_us_airport(&fp.departure) || is_us_airport(&fp.arrival));
    filed_us || airspace.any_contains(p.latitude, p.longitude)
}

/// Whether a US airport ICAO resolved from a controller callsign prefix indicates US airspace.
fn iata_is_us(iata: &IataMap, prefix: &str) -> bool {
    iata.get(prefix).is_some_and(|icao| is_us_airport(icao))
}

/// A controller is US-relevant if its callsign resolves to a US center or airport. Handles FAA radio
/// prefixes (`BOS_CTR`), bare `Zxx` centers, IATA airport prefixes (`SFO_TWR` → `KSFO`), and ICAO
/// prefixes (`KSFO_TWR`).
fn is_us_controller(callsign: &str, iata: &IataMap, airports: &AirportDb) -> bool {
    let prefix = callsign
        .split('_')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if prefix.is_empty() {
        return false;
    }
    if crate::handlers::atc::center_artcc(&prefix).is_some() {
        return true;
    }
    if is_us_airport(&prefix) || iata_is_us(iata, &prefix) {
        return true;
    }
    // US airport whose 3-letter id maps to K + id (e.g. SFO -> KSFO).
    airports.contains_key(&format!("K{prefix}"))
}

// --- feed → row conversions --------------------------------------------------------------------

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn flight_rules_char(fp: Option<&FlightPlan>) -> Option<String> {
    fp.and_then(|f| f.flight_rules.chars().next())
        .map(|c| c.to_ascii_uppercase().to_string())
}

fn flight_row(sid: i64, p: &Pilot) -> repo::FlightRow {
    let fp = p.flight_plan.as_ref();
    repo::FlightRow {
        session_id: sid,
        cid: p.cid,
        callsign: p.callsign.clone(),
        server: p.server.clone(),
        logon_time: parse_ts(&p.logon_time).unwrap_or_else(Utc::now),
        flight_rules: flight_rules_char(fp),
        departure: fp.and_then(|f| opt(&f.departure)),
        arrival: fp.and_then(|f| opt(&f.arrival)),
        alternate: fp.and_then(|f| opt(&f.alternate)),
        aircraft_short: fp.and_then(|f| opt(&f.aircraft_short)),
        aircraft_faa: fp.and_then(|f| opt(&f.aircraft_faa)),
        cruise_tas: fp.and_then(|f| f.cruise_tas.trim().parse::<i32>().ok()),
        cruise_alt: fp.and_then(|f| parse_filed_altitude(&f.altitude)),
        deptime: fp.and_then(|f| opt(&f.deptime)),
        enroute_time: fp.and_then(|f| opt(&f.enroute_time)),
        route: fp.and_then(|f| opt(&f.route)),
        remarks: fp.and_then(|f| opt(&f.remarks)),
        revision_id: fp.and_then(|f| f.revision_id),
    }
}

fn position_row(sid: i64, p: &Pilot) -> repo::PositionRow {
    repo::PositionRow {
        session_id: sid,
        lat: p.latitude as f32,
        lon: p.longitude as f32,
        altitude: p.altitude as i32,
        groundspeed: p.groundspeed.clamp(0, i16::MAX as i64) as i16,
        heading: p.heading.clamp(0, i16::MAX as i64) as i16,
        transponder: p.transponder.as_deref().and_then(opt),
        qnh_mb: p.qnh_mb.map(|v| v.clamp(0, i16::MAX as i32) as i16),
    }
}

fn prefile_row(sid: i64, pf: &Prefile) -> repo::PrefileRow {
    let fp = pf.flight_plan.as_ref();
    repo::PrefileRow {
        session_id: sid,
        cid: pf.cid,
        callsign: pf.callsign.clone(),
        departure: fp.and_then(|f| opt(&f.departure)),
        arrival: fp.and_then(|f| opt(&f.arrival)),
        alternate: fp.and_then(|f| opt(&f.alternate)),
        aircraft_short: fp.and_then(|f| opt(&f.aircraft_short)),
        route: fp.and_then(|f| opt(&f.route)),
        remarks: fp.and_then(|f| opt(&f.remarks)),
        cruise_alt: fp.and_then(|f| parse_filed_altitude(&f.altitude)),
        revision_id: fp.and_then(|f| f.revision_id),
    }
}

fn controller_row_from_ctrl(sid: i64, c: &Controller, is_atis: bool) -> repo::ControllerRow {
    repo::ControllerRow {
        session_id: sid,
        cid: c.cid,
        callsign: c.callsign.clone(),
        frequency: opt(&c.frequency),
        facility: Some(c.facility),
        rating: Some(c.rating),
        server: c.server.clone(),
        visual_range: c.visual_range,
        atis_code: None,
        logon_time: parse_ts(&c.logon_time).unwrap_or_else(Utc::now),
        is_atis,
    }
}

fn controller_row_from_atis(sid: i64, a: &Atis) -> repo::ControllerRow {
    repo::ControllerRow {
        session_id: sid,
        cid: a.cid,
        callsign: a.callsign.clone(),
        frequency: opt(&a.frequency),
        facility: Some(a.facility),
        rating: Some(a.rating),
        server: a.server.clone(),
        visual_range: a.visual_range,
        atis_code: a.atis_code.clone(),
        logon_time: parse_ts(&a.logon_time).unwrap_or_else(Utc::now),
        is_atis: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_airport_prefixes() {
        assert!(is_us_airport("KJFK"));
        assert!(is_us_airport("PANC")); // Anchorage
        assert!(is_us_airport("PHNL")); // Honolulu
        assert!(is_us_airport("TJSJ")); // San Juan
        assert!(!is_us_airport("EGLL")); // Heathrow
        assert!(!is_us_airport("CYYZ")); // Toronto
        assert!(!is_us_airport("KJF")); // too short
    }

    /// Stats scoping runs through `center_artcc`, so #482's removal of the `BDA -> ZNY` alias
    /// reaches here too: a Bermuda controller was being counted as American. The real US oceanic
    /// centres must keep counting, which is exactly why that fix filters geometry at the map's call
    /// site rather than inside `center_artcc`.
    #[test]
    fn centre_prefixes_decide_us_controllers() {
        let iata = IataMap::new();
        let airports = AirportDb::new();
        let us = |cs: &str| is_us_controller(cs, &iata, &airports);

        assert!(us("ZBW_CTR"));
        assert!(us("BOS_CTR"));
        assert!(us("ZAK_CTR"), "Oakland Oceanic is American with no polygon");
        for cs in ["SF_CTR", "SFR_CTR", "SFRO_CTR"] {
            assert!(
                us(cs),
                "San Francisco Oceanic is Oakland Oceanic (#580): {cs}"
            );
        }
        assert!(us("ZSU_CTR"), "San Juan is American with no polygon");

        assert!(!us("BDA_CTR"), "Bermuda is not a US controller");
        assert!(!us("EGLL_CTR"));
    }

    /// VATUSA/OIS#556: the Honolulu controller online as `HNL_02_CTR` was not counted as a US
    /// controller at all, because `HNL` resolved to no ARTCC.
    #[test]
    fn a_honolulu_controller_counts_as_us() {
        let iata = IataMap::new();
        let airports = AirportDb::new();

        assert!(is_us_controller("HNL_02_CTR", &iata, &airports));
        assert!(is_us_controller("HCF_CTR", &iata, &airports));
    }

    /// Every event emitted while a tick ran.
    mod capture {
        use std::collections::BTreeMap;
        use std::sync::{Arc, Mutex};

        #[derive(Debug, Clone)]
        pub(super) struct Captured {
            pub(super) level: tracing::Level,
            pub(super) message: String,
            /// Every other field, string values unquoted and `%`-formatted values as their Display.
            pub(super) fields: BTreeMap<String, String>,
        }

        pub(super) struct Recorder(pub(super) Arc<Mutex<Vec<Captured>>>);

        #[derive(Default)]
        struct Visitor {
            message: String,
            fields: BTreeMap<String, String>,
        }

        impl tracing::field::Visit for Visitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "message" {
                    self.message = value.to_owned();
                } else {
                    self.fields
                        .insert(field.name().to_owned(), value.to_owned());
                }
            }

            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.message = format!("{value:?}");
                } else {
                    self.fields
                        .insert(field.name().to_owned(), format!("{value:?}"));
                }
            }
        }

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Recorder {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let mut visitor = Visitor::default();
                event.record(&mut visitor);
                self.0
                    .lock()
                    .expect("the recorder's mutex is only held for a push")
                    .push(Captured {
                        level: *event.metadata().level(),
                        message: visitor.message,
                        fields: visitor.fields,
                    });
            }
        }
    }

    /// `tick` over an empty feed published at `source_timestamp`, with every event it emitted.
    async fn tick_capturing(
        pool: &PgPool,
        source_timestamp: &str,
        last_source: &str,
        pilots: &mut HashMap<i64, Active>,
        controllers: &mut HashMap<i64, Active>,
    ) -> (Result<Option<String>, ApiError>, Vec<capture::Captured>) {
        use tracing::instrument::WithSubscriber;
        use tracing_subscriber::layer::SubscriberExt;

        let feed = crate::feed::new_state();
        feed.write().await.snapshot = Some(Arc::new(crate::feed::Snapshot {
            fetched_at: parse_ts(source_timestamp).expect("the test passes an RFC 3339 instant"),
            source_timestamp: source_timestamp.to_owned(),
            data: crate::feed::vatsim::VatsimData::default(),
        }));
        let airspace = Boundaries::default();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber =
            tracing_subscriber::registry().with(capture::Recorder(Arc::clone(&events)));
        let result = tick(pool, &feed, &airspace, pilots, controllers, last_source)
            .with_subscriber(subscriber)
            .await;
        let captured = events
            .lock()
            .expect("the recorder's mutex is only held for a push")
            .clone();
        (result, captured)
    }

    fn gone(last_seen: &str) -> Active {
        Active {
            last_seen: parse_ts(last_seen).expect("the test passes an RFC 3339 instant"),
            last_updated: String::new(),
            revision_id: None,
        }
    }

    fn events_starting<'a>(
        events: &'a [capture::Captured],
        prefix: &str,
    ) -> Vec<&'a capture::Captured> {
        events
            .iter()
            .filter(|e| e.message.starts_with(prefix))
            .collect()
    }

    /// Make every update of `table` matching `condition` raise. `table` and `condition` are test
    /// constants, never input.
    async fn raise_on_update(pool: &PgPool, table: &str, condition: &str) {
        sqlx::raw_sql(&format!(
            "create function t937_boom() returns trigger language plpgsql as $$ \
             begin if {condition} then raise exception 'injected'; end if; return new; end $$; \
             create trigger t937_boom before update on {table} \
             for each row execute function t937_boom();"
        ))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn stop_raising(pool: &PgPool, table: &str) {
        sqlx::raw_sql(&format!(
            "drop trigger t937_boom on {table}; drop function t937_boom();"
        ))
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn a_failed_controller_close_is_kept_and_retried_next_tick(pool: PgPool) {
        let sid: i64 = 937_001;
        sqlx::query(
            "insert into stats.controller_session
                 (session_id, cid, callsign, logon_time, first_seen, last_seen)
             values ($1, 1300001, 'ZBW_CTR', '2026-01-01T10:00:00Z',
                     '2026-01-01T10:00:00Z', '2026-01-01T11:00:00Z')",
        )
        .bind(sid)
        .execute(&pool)
        .await
        .unwrap();
        let duration = || {
            sqlx::query_scalar::<_, Option<i32>>(
                "select duration_s from stats.controller_session where session_id = $1",
            )
            .bind(sid)
            .fetch_one(&pool)
        };
        assert_eq!(
            duration().await.unwrap(),
            None,
            "the row exists, not yet closed"
        );

        let mut pilots = HashMap::new();
        let mut controllers = HashMap::from([(sid, gone("2026-01-01T11:00:00Z"))]);
        raise_on_update(&pool, "stats.controller_session", "true").await;

        let t1 = "2026-01-01T11:10:00Z";
        let (result, events) = tick_capturing(&pool, t1, "", &mut pilots, &mut controllers).await;
        assert_eq!(result.unwrap().as_deref(), Some(t1));
        assert!(
            controllers.contains_key(&sid),
            "a failed close keeps the session"
        );
        assert_eq!(duration().await.unwrap(), None);
        let warns = events_starting(&events, "stats: failed to close controller session");
        assert_eq!(warns.len(), 1, "{events:?}");
        assert_eq!(warns[0].level, tracing::Level::WARN);
        assert_eq!(
            warns[0].fields.get("sid").map(String::as_str),
            Some("937001")
        );
        assert_eq!(
            warns[0].fields.get("error").map(String::as_str),
            Some("internal server error"),
            "the error's Display, not its Debug"
        );
        // Positive control: the injected exception reached the close and the recorder saw it.
        assert!(
            events.iter().any(|e| e.message == "stats db error"
                && e.fields
                    .get("error")
                    .is_some_and(|v| v.contains("injected"))),
            "{events:?}"
        );

        stop_raising(&pool, "stats.controller_session").await;
        let t2 = "2026-01-01T11:10:15Z";
        let (result, events) = tick_capturing(&pool, t2, t1, &mut pilots, &mut controllers).await;
        assert_eq!(result.unwrap().as_deref(), Some(t2));
        assert_eq!(duration().await.unwrap(), Some(3600), "11:00 - 10:00");
        assert!(
            !controllers.contains_key(&sid),
            "a closed session leaves the map"
        );
        assert!(
            events_starting(&events, "stats: failed to close").is_empty(),
            "{events:?}"
        );
    }

    #[sqlx::test]
    async fn a_failed_flight_close_is_kept_and_retried_next_tick(pool: PgPool) {
        let sid: i64 = 937_002;
        sqlx::query(
            "insert into stats.flight
                 (session_id, cid, callsign, logon_time, first_seen, last_seen, status)
             values ($1, 1300002, 'AAL937', '2026-01-01T10:00:00Z',
                     '2026-01-01T10:00:00Z', '2026-01-01T11:00:00Z', 'active')",
        )
        .bind(sid)
        .execute(&pool)
        .await
        .unwrap();
        let status = || {
            sqlx::query_scalar::<_, String>("select status from stats.flight where session_id = $1")
                .bind(sid)
                .fetch_one(&pool)
        };
        assert_eq!(
            status().await.unwrap(),
            "active",
            "the row exists, not yet closed"
        );

        let mut pilots = HashMap::from([(sid, gone("2026-01-01T11:00:00Z"))]);
        let mut controllers = HashMap::new();
        raise_on_update(&pool, "stats.flight", "new.status = 'completed'").await;

        let t1 = "2026-01-01T11:10:00Z";
        let (result, events) = tick_capturing(&pool, t1, "", &mut pilots, &mut controllers).await;
        assert_eq!(result.unwrap().as_deref(), Some(t1));
        assert!(pilots.contains_key(&sid), "a failed close keeps the flight");
        assert_eq!(status().await.unwrap(), "active");
        let warns = events_starting(&events, "stats: failed to close flight");
        assert_eq!(warns.len(), 1, "{events:?}");
        assert_eq!(warns[0].level, tracing::Level::WARN);
        assert_eq!(
            warns[0].fields.get("sid").map(String::as_str),
            Some("937002")
        );
        assert_eq!(
            warns[0].fields.get("error").map(String::as_str),
            Some("internal server error"),
            "the error's Display, not its Debug"
        );
        // Positive control: the injected exception reached the close and the recorder saw it.
        assert!(
            events.iter().any(|e| e.message == "stats db error"
                && e.fields
                    .get("error")
                    .is_some_and(|v| v.contains("injected"))),
            "{events:?}"
        );

        stop_raising(&pool, "stats.flight").await;
        let t2 = "2026-01-01T11:10:15Z";
        let (result, events) = tick_capturing(&pool, t2, t1, &mut pilots, &mut controllers).await;
        assert_eq!(result.unwrap().as_deref(), Some(t2));
        assert_eq!(status().await.unwrap(), "completed");
        assert!(!pilots.contains_key(&sid), "a closed flight leaves the map");
        assert!(
            events_starting(&events, "stats: failed to close").is_empty(),
            "{events:?}"
        );
    }

    /// Two failing closes per loop, so an early exit after the first failure is caught whatever
    /// order the map yields its sessions in: it would log one WARN, not two.
    #[sqlx::test]
    async fn a_failed_close_does_not_stop_the_other_closes_in_the_tick(pool: PgPool) {
        let controller_sids: [i64; 3] = [937_003, 937_004, 937_005];
        let flight_sids: [i64; 3] = [937_013, 937_014, 937_015];
        for sid in controller_sids {
            sqlx::query(
                "insert into stats.controller_session
                     (session_id, cid, callsign, logon_time, first_seen, last_seen)
                 values ($1, 1300003, 'ZBW_CTR', '2026-01-01T10:00:00Z',
                         '2026-01-01T10:00:00Z', '2026-01-01T11:00:00Z')",
            )
            .bind(sid)
            .execute(&pool)
            .await
            .unwrap();
        }
        for sid in flight_sids {
            sqlx::query(
                "insert into stats.flight
                     (session_id, cid, callsign, logon_time, first_seen, last_seen, status)
                 values ($1, 1300004, 'AAL937', '2026-01-01T10:00:00Z',
                         '2026-01-01T10:00:00Z', '2026-01-01T11:00:00Z', 'active')",
            )
            .bind(sid)
            .execute(&pool)
            .await
            .unwrap();
        }
        // 937_003 and 937_004 fail to close; 937_005 closes. Likewise 937_013, 937_014 and 937_015.
        raise_on_update(
            &pool,
            "stats.controller_session",
            "old.session_id in (937003, 937004)",
        )
        .await;
        sqlx::raw_sql(
            "create function t937_boom_flight() returns trigger language plpgsql as $$ \
             begin if old.session_id in (937013, 937014) then raise exception 'injected'; \
             end if; return new; end $$; \
             create trigger t937_boom_flight before update on stats.flight \
             for each row execute function t937_boom_flight();",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut controllers: HashMap<i64, Active> = controller_sids
            .iter()
            .map(|sid| (*sid, gone("2026-01-01T11:00:00Z")))
            .collect();
        let mut pilots: HashMap<i64, Active> = flight_sids
            .iter()
            .map(|sid| (*sid, gone("2026-01-01T11:00:00Z")))
            .collect();
        let (result, events) = tick_capturing(
            &pool,
            "2026-01-01T11:10:00Z",
            "",
            &mut pilots,
            &mut controllers,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");

        let duration: Option<i32> = sqlx::query_scalar(
            "select duration_s from stats.controller_session where session_id = 937005",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(duration, Some(3600), "the closing controller still closes");
        let status: String =
            sqlx::query_scalar("select status from stats.flight where session_id = 937015")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "completed", "the closing flight still closes");

        let mut kept_controllers: Vec<i64> = controllers.keys().copied().collect();
        kept_controllers.sort_unstable();
        assert_eq!(kept_controllers, vec![937_003, 937_004]);
        let mut kept_pilots: Vec<i64> = pilots.keys().copied().collect();
        kept_pilots.sort_unstable();
        assert_eq!(kept_pilots, vec![937_013, 937_014]);

        let warned = |prefix: &str| {
            let mut sids: Vec<String> = events_starting(&events, prefix)
                .iter()
                .filter_map(|e| e.fields.get("sid").cloned())
                .collect();
            sids.sort_unstable();
            sids
        };
        assert_eq!(
            warned("stats: failed to close controller session"),
            vec!["937003", "937004"],
            "{events:?}"
        );
        assert_eq!(
            warned("stats: failed to close flight"),
            vec!["937013", "937014"],
            "{events:?}"
        );
    }

    #[sqlx::test]
    async fn the_collector_logs_a_failed_reload_and_a_failed_tick_with_the_errors_display(
        pool: PgPool,
    ) {
        use tracing::instrument::WithSubscriber;
        use tracing_subscriber::layer::SubscriberExt;

        // With the schema gone, the startup reload and the tick's snapshot insert both fail.
        sqlx::raw_sql("alter schema stats rename to stats_t937")
            .execute(&pool)
            .await
            .unwrap();
        let feed = crate::feed::new_state();
        feed.write().await.snapshot = Some(Arc::new(crate::feed::Snapshot {
            fetched_at: parse_ts("2026-01-01T11:10:00Z").unwrap(),
            source_timestamp: "2026-01-01T11:10:00Z".to_owned(),
            data: crate::feed::vatsim::VatsimData::default(),
        }));
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber =
            tracing_subscriber::registry().with(capture::Recorder(Arc::clone(&events)));

        // The collector never returns; its first tick fires at once and the next 10 s later.
        let ran = tokio::time::timeout(
            Duration::from_secs(2),
            collector(pool, feed, Arc::new(Boundaries::default())).with_subscriber(subscriber),
        )
        .await;
        assert!(ran.is_err(), "the collector loops until stopped");
        let events = events
            .lock()
            .expect("the recorder's mutex is only held for a push")
            .clone();

        for message in [
            "stats: could not reload active flights",
            "stats: collection tick failed",
        ] {
            let warns: Vec<_> = events.iter().filter(|e| e.message == message).collect();
            assert_eq!(warns.len(), 1, "{message}: {events:?}");
            assert_eq!(warns[0].level, tracing::Level::WARN, "{message}");
            assert_eq!(
                warns[0].fields.get("error").map(String::as_str),
                Some("internal server error"),
                "{message}: the error's Display"
            );
        }
        // Positive control: the tick reached the database and failed on the renamed schema.
        assert!(
            events.iter().any(|e| e.message == "stats db error"
                && e.fields
                    .get("error")
                    .is_some_and(|v| v.contains("does not exist"))),
            "{events:?}"
        );
    }
}
