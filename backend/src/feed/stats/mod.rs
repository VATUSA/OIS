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
        Err(_) => tracing::warn!("stats: could not reload active flights"),
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
            Err(_) => tracing::warn!("stats: collection tick failed"),
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
    for sid in gone_pilots {
        if let Err(e) = close_flight(pool, sid).await {
            tracing::warn!(sid, error = ?e, "stats: failed to close flight");
        }
        pilots.remove(&sid);
    }

    let gone_controllers: Vec<i64> = controllers
        .iter()
        .filter(|(sid, a)| a.last_seen < cutoff && !seen_controllers.contains(sid))
        .map(|(sid, _)| *sid)
        .collect();
    for sid in gone_controllers {
        let _ = repo::close_controller(pool, sid).await;
        controllers.remove(&sid);
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
}
