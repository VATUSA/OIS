//! The sector occupancy engine's input (#721): live flights projected minute by minute over six hours,
//! as the [`Track`](super::sector_load::Track)s [`sector_loads`](super::sector_load::sector_loads) bins.
//!
//! A read-only caller of the shared trajectory model. It makes exactly the calls the map's
//! predicted-traffic projection makes (`handlers::flow::project_traffic`): `fca::route_path`, then
//! `predict::profile_from_here`, then `distance_after` / `alt_at` / `point_and_heading_at` per minute.
//! So it predicts nothing of its own and changes nothing the metering, demand or runway callers see.
//!
//! The two populations, and a flight is in exactly one:
//! - **active**: airborne (groundspeed >= 50 kt), projected from where it is now;
//! - **proposed**: on the ground or prefiled, holding a locked wheels-up, projected from it. Before
//!   wheels-up it is on the ground and in no sector, so it has no fixes. The wheels-up is the latest of
//!   its issued CFR, FCA releases and GDP slot (`repos::flow::locked_wheels_up`).
//!
//! Pure and DB-free like the rest of `feed`: the caller resolves wheels-up times and exclusions first.
//! Re-landed from the removed Airspace Monitor's projector (#701, removed in #719).

use std::collections::{HashMap, HashSet};

use crate::feed::airports::AirportDb;
use crate::feed::nav::NavData;
use crate::feed::sector_load::{Fix, HORIZON_MIN, Population};
use crate::feed::sectors::SectorTable;
use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};
use crate::feed::winds::Winds;
use crate::feed::{fca, flow as feed_flow, predict, trajectory};

/// The airborne threshold, the same one `fca::route_path` uses to pick a route from here.
pub const AIRBORNE_GS_KT: i64 = 50;
const MINUTE_MS: i64 = 60_000;

/// A flight's projected fixes, owned so the [`Track`](super::sector_load::Track)s that borrow them can be
/// built from it.
#[derive(Debug, Clone)]
pub struct OwnedTrack {
    pub id: String,
    pub population: Population,
    pub fixes: Vec<Fix>,
}

/// A lat/lon box. Only flights whose route comes near it are projected, and only fixes inside it are
/// kept, so a request for one ARTCC doesn't project the whole network.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bbox {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lon: f64,
    pub max_lon: f64,
}

impl Bbox {
    /// The box around `artcc`'s sector volumes, or `None` when it has none.
    pub fn of_artcc(table: &SectorTable, artcc: &str) -> Option<Bbox> {
        let mut points = table
            .volumes
            .iter()
            .filter(|v| v.artcc == artcc)
            .flat_map(|v| v.rings.iter().flatten());
        let first = points.next()?;
        let mut b = Bbox {
            min_lat: first[0],
            max_lat: first[0],
            min_lon: first[1],
            max_lon: first[1],
        };
        for p in points {
            b.min_lat = b.min_lat.min(p[0]);
            b.max_lat = b.max_lat.max(p[0]);
            b.min_lon = b.min_lon.min(p[1]);
            b.max_lon = b.max_lon.max(p[1]);
        }
        Some(b)
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        (self.min_lat..=self.max_lat).contains(&lat) && (self.min_lon..=self.max_lon).contains(&lon)
    }

    /// Whether the segment `a`–`b`'s own box overlaps this one: cheap, and never wrongly false.
    fn touches_segment(&self, a: [f64; 2], b: [f64; 2]) -> bool {
        a[0].min(b[0]) <= self.max_lat
            && a[0].max(b[0]) >= self.min_lat
            && a[1].min(b[1]) <= self.max_lon
            && a[1].max(b[1]) >= self.min_lon
    }

    fn touches_path(&self, path: &[[f64; 2]]) -> bool {
        match path {
            [only] => self.contains(only[0], only[1]),
            _ => path.windows(2).any(|w| self.touches_segment(w[0], w[1])),
        }
    }
}

/// Every in-scope flight's track from `now_ms` to `now_ms + HORIZON_MIN`, one fix per minute.
///
/// `wheels_up` maps a grounded callsign to its locked wheels-up (epoch ms); `excluded`
/// is every manually excluded callsign. With a `bbox`, flights whose route never comes near it are
/// skipped and fixes outside it are dropped.
#[allow(clippy::too_many_arguments)]
pub fn project_tracks(
    data: &VatsimData,
    nav: &NavData,
    airports: &AirportDb,
    profiles: &trajectory::ProfileTable,
    winds: &Winds,
    wheels_up: &HashMap<String, i64>,
    excluded: &HashSet<String>,
    now_ms: i64,
    bbox: Option<Bbox>,
) -> Vec<OwnedTrack> {
    let ctx = Ctx {
        nav,
        airports,
        profiles,
        winds,
        now_ms,
        bbox,
    };
    let pilots = data
        .pilots
        .iter()
        .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
        .filter(|p| !excluded.contains(&p.callsign))
        .filter_map(|p| {
            let fp = p.flight_plan.as_ref()?;
            if p.groundspeed >= AIRBORNE_GS_KT {
                ctx.active(p, fp)
            } else {
                let edct = *wheels_up.get(&p.callsign)?;
                ctx.proposed(&p.callsign, fp, p.latitude, p.longitude, p.heading, edct)
            }
        });
    let prefiles = data
        .prefiles
        .iter()
        .filter(|p| !excluded.contains(&p.callsign))
        .filter_map(|p| {
            let fp = p.flight_plan.as_ref()?;
            let edct = *wheels_up.get(&p.callsign)?;
            let dep = airports.get(&fp.departure.to_ascii_uppercase())?;
            ctx.proposed(&p.callsign, fp, dep.lat, dep.lon, 0, edct)
        });
    pilots.chain(prefiles).collect()
}

struct Ctx<'a> {
    nav: &'a NavData,
    airports: &'a AirportDb,
    profiles: &'a trajectory::ProfileTable,
    winds: &'a Winds,
    now_ms: i64,
    bbox: Option<Bbox>,
}

impl Ctx<'_> {
    /// An airborne flight, from where it is now, at the altitude and groundspeed it reports.
    fn active(&self, p: &Pilot, fp: &FlightPlan) -> Option<OwnedTrack> {
        let path = self.path(fp, p.latitude, p.longitude, p.heading, p.groundspeed)?;
        let fixes = self.walk(
            &path,
            fp,
            true,
            p.altitude as f64,
            p.groundspeed as f64,
            self.now_ms,
        );
        Some(OwnedTrack {
            id: p.callsign.clone(),
            population: Population::Active,
            fixes,
        })
    }

    /// A flight on the ground with a release: from its departure, starting at wheels-up — or now, if
    /// that has passed and it is still on the ground. It can't have flown the minutes it sat out.
    fn proposed(
        &self,
        id: &str,
        fp: &FlightPlan,
        lat: f64,
        lon: f64,
        heading: i64,
        edct_ms: i64,
    ) -> Option<OwnedTrack> {
        let path = self.path(fp, lat, lon, heading, 0)?;
        let fixes = self.walk(&path, fp, false, 0.0, 0.0, edct_ms.max(self.now_ms));
        Some(OwnedTrack {
            id: id.to_string(),
            population: Population::Proposed,
            fixes,
        })
    }

    /// The route ahead, or `None` when it doesn't resolve or never comes near the box.
    fn path(
        &self,
        fp: &FlightPlan,
        lat: f64,
        lon: f64,
        heading: i64,
        gs: i64,
    ) -> Option<Vec<[f64; 2]>> {
        let path = fca::route_path(
            self.nav,
            self.airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            lat,
            lon,
            heading,
            gs,
        )?;
        self.bbox
            .is_none_or(|b| b.touches_path(&path))
            .then_some(path)
    }

    /// One fix per minute from `max(now, start_ms)` to the horizon, `start_ms` being when the flight
    /// is at the start of `path` (now for an airborne flight, wheels-up for a proposed one).
    fn walk(
        &self,
        path: &[[f64; 2]],
        fp: &FlightPlan,
        airborne: bool,
        cur_alt_ft: f64,
        observed_gs_kt: f64,
        start_ms: i64,
    ) -> Vec<Fix> {
        let route_len = predict::path_len_nm(path);
        let (ty, wake) = fp.aircraft_type_wake();
        let profile = self.profiles.resolve(&ty, &wake);
        let cruise_ft = trajectory::parse_alt_ft(&fp.altitude);
        let cruise_tas =
            trajectory::capped_cruise_tas(feed_flow::parse_tas(&fp.cruise_tas), cruise_ft, profile);
        // An airborne flight's wind is taken at its altitude, as `project_traffic` does; one still on
        // the ground spends almost all of its time at cruise, so it is taken there.
        let wind_alt = if airborne { cur_alt_ft } else { cruise_ft };
        let headwind = self.winds.route_headwind(path, wind_alt);
        let vp = predict::profile_from_here(
            airborne,
            route_len,
            cur_alt_ft,
            observed_gs_kt,
            cruise_ft,
            cruise_tas,
            self.airports,
            &fp.arrival,
            profile,
            headwind,
        );

        let end_ms = self.now_ms + HORIZON_MIN * MINUTE_MS;
        let mut fixes = Vec::new();
        let mut t = self.now_ms.max(start_ms);
        while t <= end_ms {
            let d = vp.distance_after(route_len, (t - start_ms) as f64 / 1000.0);
            let (pos, _) = fca::point_and_heading_at(path, (route_len - d).max(0.0));
            if self.bbox.is_none_or(|b| b.contains(pos[0], pos[1])) {
                fixes.push(Fix {
                    t_ms: t,
                    lat: pos[0],
                    lon: pos[1],
                    alt_ft: Some(vp.alt_at(d)),
                });
            }
            if d <= 0.0 {
                break; // landed
            }
            t += MINUTE_MS;
        }
        fixes
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap, HashSet};

    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::feed::airports::Airport;
    use crate::feed::sector_load::BIN_MIN;
    use crate::feed::sector_load::{Track, sector_loads};
    use crate::feed::sectors::SectorVolume;
    use crate::feed::trajectory::ProfileTable;
    use crate::feed::vatsim::{Prefile, VatsimData};

    /// 1407Z; the Monitor's first bin starts at 1400.
    fn now() -> i64 {
        Utc.with_ymd_and_hms(2026, 10, 4, 14, 7, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn airports() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ])
    }

    /// KJFK -> KDCA via a route the bundled nav db resolves (as `handlers::flow`'s projection tests).
    fn plan() -> FlightPlan {
        FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            route: "RBV WHITE SIE".into(),
            aircraft_short: "B738".into(),
            cruise_tas: "440".into(),
            altitude: "35000".into(),
            ..Default::default()
        }
    }

    /// Just south of KJFK, tracking SW; airborne at `gs` >= 50, on the ground below it.
    fn pilot(callsign: &str, gs: i64) -> Pilot {
        Pilot {
            callsign: callsign.into(),
            latitude: 40.2,
            longitude: -74.0,
            altitude: if gs >= AIRBORNE_GS_KT { 24_000 } else { 13 },
            groundspeed: gs,
            heading: 220,
            flight_plan: Some(plan()),
            ..Default::default()
        }
    }

    fn prefile(callsign: &str) -> Prefile {
        Prefile {
            callsign: callsign.into(),
            flight_plan: Some(plan()),
            ..Default::default()
        }
    }

    fn project(
        data: &VatsimData,
        releases: &[(&str, i64)],
        excluded: &[&str],
        bbox: Option<Bbox>,
    ) -> Vec<OwnedTrack> {
        let releases: HashMap<String, i64> =
            releases.iter().map(|(c, t)| (c.to_string(), *t)).collect();
        let excluded: HashSet<String> = excluded.iter().map(|c| c.to_string()).collect();
        project_tracks(
            data,
            &NavData::load(),
            &airports(),
            &ProfileTable::default(),
            &Winds::default(),
            &releases,
            &excluded,
            now(),
            bbox,
        )
    }

    fn pilots(pilots: Vec<Pilot>) -> VatsimData {
        VatsimData {
            pilots,
            ..Default::default()
        }
    }

    /// AC1: a flight crossing a sector counts in exactly the bins whose minutes it is inside — never
    /// before it arrives or after it leaves. The sector is a latitude band over a stretch of the
    /// flight's south-westbound route, so the flight is inside for one contiguous window.
    #[test]
    fn a_flight_counts_in_exactly_the_bins_it_is_inside() {
        let tracks = project(&pilots(vec![pilot("AAL1", 400)]), &[], &[], None);
        let fixes = &tracks[0].fixes;
        assert!(
            fixes.len() > 25,
            "the fixture must fly long enough: {}",
            fixes.len()
        );
        let (north, south) = (fixes[10].lat, fixes[20].lat);
        let band = SectorVolume {
            rings: vec![vec![
                [south, -80.0],
                [south, -70.0],
                [north, -70.0],
                [north, -80.0],
                [south, -80.0],
            ]],
            base_alt_ft: 0,
            top_alt_ft: 60_000,
            ..crate::feed::sectors::tests::volume("ZDC", "02001")
        };
        let inside = |f: &Fix| band.contains(f.lat, f.lon, f.alt_ft);
        let first_ms = now() - now().rem_euclid(BIN_MIN * MINUTE_MS);
        let expected: BTreeSet<i64> = fixes
            .iter()
            .filter(|f| inside(f))
            .map(|f| (f.t_ms - first_ms) / (BIN_MIN * MINUTE_MS))
            .collect();
        assert!(
            !expected.is_empty(),
            "the band must catch part of the route"
        );

        let table = SectorTable {
            volumes: vec![band],
        };
        let borrowed = [Track {
            id: &tracks[0].id,
            population: tracks[0].population,
            fixes,
        }];
        let loads = sector_loads(&table, &borrowed, now());
        let counted: BTreeSet<i64> = loads[0]
            .bins
            .iter()
            .enumerate()
            .filter(|(_, b)| b.active > 0)
            .map(|(i, _)| i as i64)
            .collect();
        assert_eq!(counted, expected);
        assert!(!counted.contains(&0), "it starts outside the sector");
        assert!(
            !counted.contains(&(HORIZON_MIN / BIN_MIN - 1)),
            "it has landed long before the last bin"
        );
    }

    /// AC2: a proposed flight is on the ground until its wheels-up, so its track starts then, at its
    /// departure — not now.
    #[test]
    fn a_proposed_track_starts_at_wheels_up() {
        let edct = now() + 40 * MINUTE_MS;
        let data = VatsimData {
            prefiles: vec![prefile("DAL2")],
            ..Default::default()
        };
        let tracks = project(&data, &[("DAL2", edct)], &[], None);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].population, Population::Proposed);
        let first = tracks[0].fixes[0];
        assert_eq!(first.t_ms, edct, "no fix before wheels-up");
        assert!(
            (first.lat - 40.64).abs() < 0.05 && (first.lon + 73.78).abs() < 0.05,
            "it starts at KJFK: {first:?}"
        );
    }

    /// A release whose wheels-up has passed, for a flight still on the ground (a pilot at the gate, or
    /// a prefile that never connected), departs no earlier than now: projecting it from the missed
    /// wheels-up put it half an hour down its route, counted in sectors it isn't in (#701 review).
    #[test]
    fn a_late_proposed_flight_departs_now_not_at_its_missed_wheels_up() {
        let edct = now() - 30 * MINUTE_MS;
        // (feed, where the flight is now: a prefile at its departure, a pilot where it reports)
        for (data, (lat, lon)) in [
            (
                VatsimData {
                    prefiles: vec![prefile("LATE")],
                    ..Default::default()
                },
                (40.64, -73.78),
            ),
            (pilots(vec![pilot("LATE", 0)]), (40.2, -74.0)),
        ] {
            let tracks = project(&data, &[("LATE", edct)], &[], None);
            assert_eq!(tracks[0].population, Population::Proposed);
            let first = tracks[0].fixes[0];
            assert_eq!(first.t_ms, now());
            assert!(
                (first.lat - lat).abs() < 0.05 && (first.lon - lon).abs() < 0.05,
                "still on the ground where it is, not down its route: {first:?}"
            );
            assert_eq!(first.alt_ft, Some(0.0), "still on the ground: {first:?}");
        }
    }

    /// A flight is in exactly one population: airborne at 50 kt and over is active whether or not it
    /// holds a release; below it, only a release puts it on the Monitor.
    #[test]
    fn populations_follow_groundspeed_and_releases() {
        let data = VatsimData {
            pilots: vec![
                pilot("FAST", AIRBORNE_GS_KT),
                pilot("SLOW", AIRBORNE_GS_KT - 1),
                pilot("IDLE", 0),
            ],
            prefiles: vec![prefile("NOREL")],
            ..Default::default()
        };
        let release = now() + 10 * MINUTE_MS;
        let tracks = project(&data, &[("FAST", release), ("SLOW", release)], &[], None);
        let by_id: HashMap<&str, Population> = tracks
            .iter()
            .map(|t| (t.id.as_str(), t.population))
            .collect();
        assert_eq!(
            by_id,
            HashMap::from([("FAST", Population::Active), ("SLOW", Population::Proposed)])
        );
    }

    #[test]
    fn excluded_flights_are_not_projected() {
        let tracks = project(&pilots(vec![pilot("BOGUS", 400)]), &[], &["BOGUS"], None);
        assert!(tracks.is_empty());
    }

    /// The cost guard: a flight whose route never comes near the ARTCC is not projected at all.
    #[test]
    fn a_route_nowhere_near_the_box_is_skipped() {
        let seattle = Bbox {
            min_lat: 46.0,
            max_lat: 49.0,
            min_lon: -124.0,
            max_lon: -120.0,
        };
        assert!(project(&pilots(vec![pilot("AAL1", 400)]), &[], &[], Some(seattle)).is_empty());
        let east_coast = Bbox {
            min_lat: 38.0,
            max_lat: 41.0,
            min_lon: -78.0,
            max_lon: -73.0,
        };
        assert_eq!(
            project(
                &pilots(vec![pilot("AAL1", 400)]),
                &[],
                &[],
                Some(east_coast)
            )
            .len(),
            1
        );
    }
}
