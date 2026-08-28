//! Taxi-time monitor. Stateful across feed polls: it watches each departure roll off its
//! field — from ~7 kt (start of taxi) until airborne (>60 kt or a >100 ft climb) — and
//! records the elapsed time as a sample. Averages per airport, with a trend, drive the
//! Taxi Monitor page. Ported from vatflow's taxi-monitor-bg.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use super::airports::AirportDb;
use super::flow::gc_dist;
use super::vatsim::VatsimData;

const GS_START: i64 = 7; // kt — taxi/roll has begun
const GS_STOP: i64 = 60; // kt — airborne
const ALT_CLIMB_FT: i64 = 100;
const DEP_PROX_NM: f64 = 15.0; // must be this close to the departure field
const MIN_SAMPLE_MS: i64 = 3_000;
const MAX_SAMPLE_MS: i64 = 60 * 60_000; // ignore implausibly long taxis
const SESSION_MAX_AGE_MS: i64 = 90 * 60_000;
const SAMPLE_MAX_AGE_MS: i64 = 3 * 60 * 60_000; // keep 3h of history
const SAMPLES_PER_AIRPORT: usize = 400;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Watching,
    Rolling,
}

/// A departure being timed.
pub struct TaxiSession {
    dep: String,
    phase: Phase,
    first_seen_ms: i64,
    start_ms: Option<i64>,
    base_alt: i64,
}

/// A completed taxi observation.
#[derive(Clone, Copy)]
pub struct TaxiSample {
    pub end_ms: i64,
    pub duration_ms: i64,
}

/// Aggregate taxi stats for one airport.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaxiStats {
    pub icao: String,
    /// Average taxi time (minutes) over the last hour; null with no samples.
    pub avg_min: Option<i64>,
    /// Completed taxis in the last hour.
    pub volume: usize,
    /// `increasing` | `decreasing` | `steady` (last 5 vs previous 5).
    pub trend: String,
    /// Total samples retained (up to 3h).
    pub sample_count: usize,
    /// Most recent taxi time in seconds; null with no samples.
    pub last_sec: Option<i64>,
}

/// A departure currently being timed at a field.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaxiActive {
    pub callsign: String,
    pub dest: String,
    pub gs: i64,
    pub alt: i64,
    /// `watching` (not yet moving) | `rolling` (taxi in progress).
    pub phase: String,
    /// When the roll began, for a live client-side timer; null while watching.
    pub rolling_since: Option<DateTime<Utc>>,
}

/// The Taxi Monitor view for one field: stats plus the in-progress departures.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaxiField {
    pub icao: String,
    pub avg_min: Option<i64>,
    pub volume: usize,
    pub trend: String,
    pub sample_count: usize,
    pub last_sec: Option<i64>,
    pub active: Vec<TaxiActive>,
}

/// Advance the taxi state machine with a fresh feed snapshot.
pub fn process(
    sessions: &mut HashMap<String, TaxiSession>,
    samples: &mut HashMap<String, Vec<TaxiSample>>,
    airports: &AirportDb,
    data: &VatsimData,
    now: DateTime<Utc>,
) {
    let now_ms = now.timestamp_millis();
    let mut seen: HashSet<String> = HashSet::new();
    let mut to_remove: Vec<String> = Vec::new();
    let mut new_samples: Vec<(String, TaxiSample)> = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let dep = fp.departure.to_ascii_uppercase();
        if dep.is_empty() {
            continue;
        }
        let gs = p.groundspeed;
        let alt = p.altitude;

        // Existing session for this flight's field: advance it (regardless of distance, so
        // a fast departure that has already flown past the field still gets finished).
        if let Some(s) = sessions.get_mut(&p.callsign).filter(|s| s.dep == dep) {
            seen.insert(p.callsign.clone());
            if s.phase == Phase::Watching && gs > GS_START {
                s.phase = Phase::Rolling;
                s.start_ms = Some(now_ms);
                s.base_alt = alt;
            }
            if s.phase == Phase::Rolling {
                let climbed = alt >= s.base_alt + ALT_CLIMB_FT;
                if gs > GS_STOP || climbed {
                    if let Some(start) = s.start_ms {
                        let start_ms = if now_ms - start < MIN_SAMPLE_MS && s.first_seen_ms < start
                        {
                            s.first_seen_ms
                        } else {
                            start
                        };
                        let dur = now_ms - start_ms;
                        if (MIN_SAMPLE_MS..=MAX_SAMPLE_MS).contains(&dur) {
                            new_samples.push((
                                s.dep.clone(),
                                TaxiSample {
                                    end_ms: now_ms,
                                    duration_ms: dur,
                                },
                            ));
                        }
                    }
                    to_remove.push(p.callsign.clone());
                }
            }
            continue;
        }

        // No session yet — only start one for a genuine departure sitting at its field.
        let Some((dlat, dlon)) = airports.get(&dep).copied() else {
            continue;
        };
        if gc_dist(p.latitude, p.longitude, dlat, dlon) > DEP_PROX_NM {
            continue;
        }
        if gs > GS_STOP && alt > 500 {
            continue; // already climbing out — missed the taxi
        }
        // Guard against a turnaround aircraft that's actually arriving at its destination.
        let dest = fp.arrival.to_ascii_uppercase();
        if gs <= GS_STOP
            && let Some((alat, alon)) = airports.get(&dest).copied()
            && gc_dist(p.latitude, p.longitude, alat, alon) < 5.0
        {
            continue;
        }
        sessions.insert(
            p.callsign.clone(),
            TaxiSession {
                dep,
                phase: Phase::Watching,
                first_seen_ms: now_ms,
                start_ms: None,
                base_alt: alt,
            },
        );
        seen.insert(p.callsign.clone());
    }

    for cs in to_remove {
        sessions.remove(&cs);
    }
    // Drop sessions for flights that vanished from the feed, or that have lingered too long.
    sessions.retain(|cs, s| seen.contains(cs) && now_ms - s.first_seen_ms < SESSION_MAX_AGE_MS);

    for (dep, sample) in new_samples {
        let list = samples.entry(dep).or_default();
        list.push(sample);
        if list.len() > SAMPLES_PER_AIRPORT {
            let excess = list.len() - SAMPLES_PER_AIRPORT;
            list.drain(0..excess);
        }
    }
    for list in samples.values_mut() {
        list.retain(|s| now_ms - s.end_ms < SAMPLE_MAX_AGE_MS);
    }
    samples.retain(|_, v| !v.is_empty());
}

/// Compute aggregate stats for one airport from its retained samples.
pub fn stats(icao: &str, samples: Option<&Vec<TaxiSample>>, now: DateTime<Utc>) -> TaxiStats {
    let now_ms = now.timestamp_millis();
    let empty = Vec::new();
    let all = samples.unwrap_or(&empty);

    // Newest first.
    let mut sorted: Vec<TaxiSample> = all.clone();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.end_ms));

    let hour: Vec<&TaxiSample> = sorted
        .iter()
        .filter(|s| now_ms - s.end_ms < 60 * 60_000)
        .collect();

    let avg_min = if hour.is_empty() {
        None
    } else {
        let mean_ms = hour.iter().map(|s| s.duration_ms).sum::<i64>() / hour.len() as i64;
        Some(((mean_ms as f64) / 60_000.0).round() as i64)
    };

    let mean = |slice: &[TaxiSample]| -> f64 {
        slice.iter().map(|s| s.duration_ms as f64).sum::<f64>() / slice.len().max(1) as f64
    };
    let trend = if sorted.len() >= 10 {
        let last5 = mean(&sorted[0..5]);
        let prev5 = mean(&sorted[5..10]);
        let delta_min = (last5 - prev5) / 60_000.0;
        if delta_min >= 2.0 {
            "increasing"
        } else if delta_min <= -2.0 {
            "decreasing"
        } else {
            "steady"
        }
    } else {
        "steady"
    }
    .to_string();

    TaxiStats {
        icao: icao.to_string(),
        avg_min,
        volume: hour.len(),
        trend,
        sample_count: sorted.len(),
        last_sec: sorted.first().map(|s| (s.duration_ms / 1000).max(0)),
    }
}

/// The departures currently being timed at `icao`, joined with their live feed position.
pub fn active(
    sessions: &HashMap<String, TaxiSession>,
    data: &VatsimData,
    icao: &str,
    now: DateTime<Utc>,
) -> Vec<TaxiActive> {
    let mut out: Vec<TaxiActive> = sessions
        .iter()
        .filter(|(_, s)| s.dep == icao)
        .map(|(cs, s)| {
            let pilot = data.pilots.iter().find(|p| &p.callsign == cs);
            let dest = pilot
                .and_then(|p| p.flight_plan.as_ref())
                .map(|fp| fp.arrival.to_ascii_uppercase())
                .unwrap_or_default();
            let (phase, rolling_since) = match s.phase {
                Phase::Watching => ("watching", None),
                Phase::Rolling => (
                    "rolling",
                    s.start_ms.and_then(DateTime::from_timestamp_millis),
                ),
            };
            TaxiActive {
                callsign: cs.clone(),
                dest,
                gs: pilot.map(|p| p.groundspeed).unwrap_or(0),
                alt: pilot.map(|p| p.altitude).unwrap_or(0),
                phase: phase.to_string(),
                rolling_since,
            }
        })
        .collect();
    // Rolling first (most interesting), then by callsign.
    out.sort_by(|a, b| {
        b.rolling_since
            .cmp(&a.rolling_since)
            .then(a.callsign.cmp(&b.callsign))
    });
    let _ = now;
    out
}

/// The full Taxi Monitor view for one field: aggregate stats plus in-progress departures.
pub fn field(
    sessions: &HashMap<String, TaxiSession>,
    samples: Option<&Vec<TaxiSample>>,
    data: &VatsimData,
    icao: &str,
    now: DateTime<Utc>,
) -> TaxiField {
    let s = stats(icao, samples, now);
    TaxiField {
        icao: s.icao,
        avg_min: s.avg_min,
        volume: s.volume,
        trend: s.trend,
        sample_count: s.sample_count,
        last_sec: s.last_sec,
        active: active(sessions, data, icao, now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn snapshot(gs: i64, alt: i64) -> VatsimData {
        VatsimData {
            pilots: vec![Pilot {
                callsign: "AAL1".into(),
                latitude: 40.0,
                longitude: -74.0,
                altitude: alt,
                groundspeed: gs,
                heading: 0,
                flight_plan: Some(FlightPlan {
                    departure: "KAAA".into(),
                    arrival: "KBBB".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn airports() -> AirportDb {
        HashMap::from([("KAAA".to_string(), (40.0, -74.0))])
    }

    #[test]
    fn times_a_full_departure_roll() {
        let ap = airports();
        let mut sessions = HashMap::new();
        let mut samples = HashMap::new();

        // At the gate (watching), then taxiing (rolling starts), then airborne (finish).
        process(&mut sessions, &mut samples, &ap, &snapshot(0, 0), t(0));
        process(&mut sessions, &mut samples, &ap, &snapshot(20, 0), t(30));
        process(&mut sessions, &mut samples, &ap, &snapshot(80, 400), t(120));

        let s = samples.get("KAAA").expect("a KAAA sample");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].duration_ms, 90_000); // rolled at t=30, airborne at t=120
        assert!(sessions.is_empty()); // session closed out

        let stats = stats("KAAA", samples.get("KAAA"), t(120));
        assert_eq!(stats.volume, 1);
        assert_eq!(stats.avg_min, Some(2)); // 90s -> 1.5 -> 2
        assert_eq!(stats.last_sec, Some(90));
    }

    #[test]
    fn ignores_an_aircraft_already_airborne_when_first_seen() {
        let ap = airports();
        let mut sessions = HashMap::new();
        let mut samples = HashMap::new();
        // First sighting is already climbing out — no taxi to time.
        process(&mut sessions, &mut samples, &ap, &snapshot(200, 3000), t(0));
        assert!(sessions.is_empty());
        assert!(samples.is_empty());
    }

    #[test]
    fn empty_stats_when_no_samples() {
        let s = stats("KAAA", None, t(0));
        assert_eq!(s.avg_min, None);
        assert_eq!(s.volume, 0);
        assert_eq!(s.trend, "steady");
    }
}
