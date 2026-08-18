//! Reconstruct a point-in-time `VatsimData` snapshot from the persisted stats tables, so the
//! live feed's pure compute functions (flow / runway / atc / traffic) can be replayed against a
//! past instant `T`. This is the backend half of the historical ("time-machine") dashboard: the
//! collector persisted the network over time, and `reconstruct_at` rebuilds the in-memory shape
//! the live handlers already know how to meter.
//!
//! A flight is "present at T" when its session overlaps T (`first_seen <= T <= last_seen`) and it
//! has at least one stored position at or before T — the nearest such position gives its location,
//! exactly like the map replay. Controllers/ATIS use the same overlap test. Winds/nav are the
//! *current* runtime data (not persisted per-tick), so reconstructed ETAs aren't bit-exact to what
//! was shown live — acceptable for a debrief view.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::feed::airports::AirportDb;
use crate::feed::taxi::{self, TaxiField, TaxiSample, TaxiSession};
use crate::feed::vatsim::{Atis, Controller, FlightPlan, Pilot, Prefile, VatsimData};

#[derive(sqlx::FromRow)]
struct PilotRow {
    callsign: String,
    cid: i32,
    departure: Option<String>,
    arrival: Option<String>,
    aircraft_short: Option<String>,
    aircraft_faa: Option<String>,
    flight_rules: Option<String>,
    route: Option<String>,
    cruise_alt: Option<i32>,
    cruise_tas: Option<i32>,
    deptime: Option<String>,
    alternate: Option<String>,
    lat: f32,
    lon: f32,
    altitude: i32,
    groundspeed: i16,
    heading: i16,
}

impl PilotRow {
    fn into_pilot(self) -> Pilot {
        let flight_plan = FlightPlan {
            departure: self.departure.unwrap_or_default(),
            arrival: self.arrival.unwrap_or_default(),
            aircraft_short: self.aircraft_short.unwrap_or_default(),
            aircraft_faa: self.aircraft_faa.unwrap_or_default(),
            flight_rules: self.flight_rules.unwrap_or_default(),
            route: self.route.unwrap_or_default(),
            // The compute functions parse these back out of the string forms the live feed uses.
            altitude: self.cruise_alt.map(|a| a.to_string()).unwrap_or_default(),
            cruise_tas: self.cruise_tas.map(|t| t.to_string()).unwrap_or_default(),
            deptime: self.deptime.unwrap_or_default(),
            alternate: self.alternate.unwrap_or_default(),
            ..Default::default()
        };
        Pilot {
            callsign: self.callsign,
            cid: self.cid,
            latitude: self.lat as f64,
            longitude: self.lon as f64,
            altitude: self.altitude as i64,
            groundspeed: self.groundspeed as i64,
            heading: self.heading as i64,
            flight_plan: Some(flight_plan),
            ..Default::default()
        }
    }
}

#[derive(sqlx::FromRow)]
struct ControllerRow {
    callsign: String,
    cid: i32,
    frequency: Option<String>,
    facility: Option<i32>,
    rating: Option<i32>,
    atis_code: Option<String>,
    is_atis: bool,
}

/// Rebuild the network snapshot at instant `at` from `stats.flight` + `stats.position` (pilots) and
/// `stats.controller_session` (controllers/ATIS). Prefiles are not reconstructed (they carry no
/// position and aren't meaningful at a past instant), so `prefiles` is always empty.
pub async fn reconstruct_at(pool: &PgPool, at: DateTime<Utc>) -> Result<VatsimData, ApiError> {
    // Pilots active at T, each with its nearest stored position at or before T (lateral join on the
    // `(session_id, ts desc)` index). Flights with no position by T are dropped — they aren't
    // placeable on the map, matching the replay.
    let pilots = sqlx::query_as::<_, PilotRow>(
        "select f.callsign, f.cid, f.departure, f.arrival, f.aircraft_short, f.aircraft_faa,
                f.flight_rules, f.route, f.cruise_alt, f.cruise_tas, f.deptime, f.alternate,
                p.lat, p.lon, p.altitude, p.groundspeed, p.heading
         from stats.flight f
         join lateral (
            select lat, lon, altitude, groundspeed, heading
            from stats.position
            where session_id = f.session_id and ts <= $1
            order by ts desc limit 1
         ) p on true
         where f.status <> 'prefiled' and f.first_seen <= $1 and f.last_seen >= $1",
    )
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(db)?
    .into_iter()
    .map(PilotRow::into_pilot)
    .collect();

    // Controllers + ATIS active at T.
    let sessions = sqlx::query_as::<_, ControllerRow>(
        "select callsign, cid, frequency, facility, rating, atis_code, is_atis
         from stats.controller_session
         where first_seen <= $1 and last_seen >= $1",
    )
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(db)?;

    let mut controllers: Vec<Controller> = Vec::new();
    let mut atis: Vec<Atis> = Vec::new();
    for s in sessions {
        if s.is_atis {
            atis.push(Atis {
                callsign: s.callsign,
                frequency: s.frequency.unwrap_or_default(),
                atis_code: s.atis_code,
                text_atis: None,
                cid: s.cid,
                name: String::new(),
                facility: s.facility.unwrap_or_default(),
                rating: s.rating.unwrap_or_default(),
                server: None,
                visual_range: None,
                logon_time: String::new(),
                last_updated: String::new(),
            });
        } else {
            controllers.push(Controller {
                callsign: s.callsign,
                frequency: s.frequency.unwrap_or_default(),
                facility: s.facility.unwrap_or_default(),
                rating: s.rating.unwrap_or_default(),
                cid: s.cid,
                name: String::new(),
                server: None,
                visual_range: None,
                logon_time: String::new(),
                last_updated: String::new(),
            });
        }
    }

    Ok(VatsimData {
        general: Default::default(),
        pilots,
        prefiles: Vec::<Prefile>::new(),
        controllers,
        atis,
    })
}

/// How far back to replay the taxi machine — covers the full sample-retention window
/// (`SAMPLE_MAX_AGE_MS` = 3h in `taxi.rs`) so the reconstructed averages/trend match live.
const TAXI_LOOKBACK_H: i64 = 3;

#[derive(sqlx::FromRow)]
struct TaxiPosRow {
    callsign: String,
    arrival: Option<String>,
    ts: DateTime<Utc>,
    lat: f32,
    lon: f32,
    altitude: i32,
    groundspeed: i16,
}

/// Reconstruct the Taxi Monitor view for `icao` at instant `at` by replaying the stored position
/// stream of that field's departures through the SAME `taxi::process` state machine the live feed
/// drives — so the averages/trend/active list are computed by the canonical code, not a
/// reimplementation. `icao` must be uppercase.
///
/// The stream is grouped into per-tick snapshots (positions sharing a `ts`) and fed in time order.
/// For the recent hour the collector's 15s samples are full-resolution, so the live view is
/// reproduced faithfully; older samples may be thinned by compaction (they only affect the trend and
/// total count, not the last-hour average/volume).
pub async fn taxi_field_at(
    pool: &PgPool,
    airports: &AirportDb,
    icao: &str,
    at: DateTime<Utc>,
) -> Result<TaxiField, ApiError> {
    let from = at - Duration::hours(TAXI_LOOKBACK_H);
    let rows = sqlx::query_as::<_, TaxiPosRow>(
        "select f.callsign, f.arrival, p.ts, p.lat, p.lon, p.altitude, p.groundspeed
         from stats.position p
         join stats.flight f on f.session_id = p.session_id
         where upper(f.departure) = $1 and f.status <> 'prefiled'
           and p.ts >= $2 and p.ts <= $3
         order by p.ts, f.callsign",
    )
    .bind(icao)
    .bind(from)
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(db)?;

    let mut sessions: HashMap<String, TaxiSession> = HashMap::new();
    let mut samples: HashMap<String, Vec<TaxiSample>> = HashMap::new();
    let mut last_data = VatsimData::default();

    // Group consecutive rows into ticks (same `ts`) and advance the machine one tick at a time.
    let mut i = 0;
    while i < rows.len() {
        let ts = rows[i].ts;
        let mut pilots: Vec<Pilot> = Vec::new();
        while i < rows.len() && rows[i].ts == ts {
            let r = &rows[i];
            pilots.push(Pilot {
                callsign: r.callsign.clone(),
                latitude: r.lat as f64,
                longitude: r.lon as f64,
                altitude: r.altitude as i64,
                groundspeed: r.groundspeed as i64,
                heading: 0,
                flight_plan: Some(FlightPlan {
                    departure: icao.to_string(),
                    arrival: r.arrival.clone().unwrap_or_default(),
                    ..Default::default()
                }),
                ..Default::default()
            });
            i += 1;
        }
        let data = VatsimData {
            pilots,
            ..Default::default()
        };
        taxi::process(&mut sessions, &mut samples, airports, &data, ts);
        last_data = data;
    }

    Ok(taxi::field(
        &sessions,
        samples.get(icao),
        &last_data,
        icao,
        at,
    ))
}

fn db(e: sqlx::Error) -> ApiError {
    tracing::warn!(error = %e, "stats reconstruct db error");
    ApiError::Internal
}
