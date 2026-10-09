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
use crate::feed::flow::gc_dist;
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

    /// Whether the two boxes share any point.
    fn overlaps(&self, other: &Bbox) -> bool {
        self.min_lat <= other.max_lat
            && self.max_lat >= other.min_lat
            && self.min_lon <= other.max_lon
            && self.max_lon >= other.min_lon
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
        every_minute: false,
        #[cfg(test)]
        evaluated: Default::default(),
    };
    ctx.project(data, wheels_up, excluded)
}

impl Ctx<'_> {
    fn project(
        &self,
        data: &VatsimData,
        wheels_up: &HashMap<String, i64>,
        excluded: &HashSet<String>,
    ) -> Vec<OwnedTrack> {
        let (ctx, airports) = (self, self.airports);
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
}

struct Ctx<'a> {
    nav: &'a NavData,
    airports: &'a AirportDb,
    profiles: &'a trajectory::ProfileTable,
    winds: &'a Winds,
    now_ms: i64,
    bbox: Option<Bbox>,
    /// Resolve every minute even with a box, as the walk did before it skipped the minutes outside
    /// it; only the tests that pin the two to the same output set it.
    every_minute: bool,
    /// How many minutes were resolved with `distance_after`, for the tests that show minutes are skipped.
    #[cfg(test)]
    evaluated: std::cell::Cell<usize>,
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

        let first_ms = self.now_ms.max(start_ms);
        let end_ms = self.now_ms + HORIZON_MIN * MINUTE_MS;
        let minutes = if first_ms > end_ms {
            0
        } else {
            ((end_ms - first_ms) / MINUTE_MS + 1) as usize
        };
        let mut minute = Minutes {
            vp: &vp,
            route_len,
            start_ms,
            first_ms,
            d: vec![None; minutes],
            #[cfg(test)]
            evaluated: &self.evaluated,
        };
        let mut fixes = Vec::new();
        let windows = match self.bbox {
            Some(b) if path.len() >= 2 && !self.every_minute => along_route_windows(path, &b),
            // No box keeps every minute, so there is nothing to skip.
            _ => vec![(f64::NEG_INFINITY, f64::INFINITY)],
        };
        let mut i = 0;
        for (lo, hi) in windows {
            // The minutes are in route order, so the ones in this window are a run starting at the
            // first whose position is not short of it.
            i = minute.first_reaching(i, lo);
            while i < minutes {
                let (d, s) = minute.at(i);
                if s > hi {
                    break;
                }
                let (pos, _) = fca::point_and_heading_at(path, s);
                if self.bbox.is_none_or(|b| b.contains(pos[0], pos[1])) {
                    fixes.push(Fix {
                        t_ms: minute.t_ms(i),
                        lat: pos[0],
                        lon: pos[1],
                        alt_ft: Some(vp.alt_at(d)),
                    });
                }
                if d <= 0.0 {
                    return fixes; // landed
                }
                i += 1;
            }
        }
        fixes
    }
}

/// One track's minutes, `first_ms` and every minute after it to the horizon, each resolved to its
/// distance-to-destination `d` (the shared model's `distance_after`, the cost of a projection) at most
/// once and only when asked for.
///
/// `d` never increases from one minute to the next, so `s`, how far along the path the flight is, never
/// decreases: `distance_after` bisects the same tree for every elapsed time, and a later time leaves it
/// at or below where an earlier one did. That is what lets [`Self::first_reaching`] bisect the minutes.
struct Minutes<'a> {
    vp: &'a trajectory::VerticalProfile,
    route_len: f64,
    start_ms: i64,
    first_ms: i64,
    d: Vec<Option<f64>>,
    #[cfg(test)]
    evaluated: &'a std::cell::Cell<usize>,
}

impl Minutes<'_> {
    fn t_ms(&self, i: usize) -> i64 {
        self.first_ms + i as i64 * MINUTE_MS
    }

    /// Minute `i`'s `(d, s)`, exactly as the every-minute walk computes them.
    fn at(&mut self, i: usize) -> (f64, f64) {
        let d = match self.d[i] {
            Some(d) => d,
            None => {
                #[cfg(test)]
                self.evaluated.set(self.evaluated.get() + 1);
                let elapsed_sec = (self.t_ms(i) - self.start_ms) as f64 / 1000.0;
                let d = self.vp.distance_after(self.route_len, elapsed_sec);
                self.d[i] = Some(d);
                d
            }
        };
        (d, (self.route_len - d).max(0.0))
    }

    /// The first minute from `from` on whose `s` is at least `lo`, or the minute count when none is.
    fn first_reaching(&mut self, from: usize, lo: f64) -> usize {
        if lo == f64::NEG_INFINITY {
            return from;
        }
        let (mut a, mut b) = (from, self.d.len());
        while a < b {
            let mid = a + (b - a) / 2;
            if self.at(mid).1 < lo {
                a = mid + 1;
            } else {
                b = mid;
            }
        }
        a
    }
}

/// The stretches of `path`, as along-route distances `[lo, hi]` (nm from its start), where a point could
/// be inside `bbox`; sorted, disjoint, and never too narrow, so a minute outside all of them is outside
/// the box and needs no `distance_after`.
///
/// They follow [`fca::point_and_heading_at`]: a distance in `(acc, acc + seg]` is placed on that leg (the
/// first leg takes everything up to its end, the last everything past the path's end), on the great
/// circle between its ends, `fca::slerp`'d by its share of the leg. Each leg is cut into pieces of at
/// most [`PIECE_NM`], since a filed route can have legs hundreds of miles long, and a piece's stretch
/// is kept when [`arc_box`] around it meets the box.
fn along_route_windows(path: &[[f64; 2]], bbox: &Bbox) -> Vec<(f64, f64)> {
    let legs = path.len() - 1;
    let mut windows: Vec<(f64, f64)> = Vec::new();
    let mut acc = 0.0;
    for (k, w) in path.windows(2).enumerate() {
        let seg = gc_dist(w[0][0], w[0][1], w[1][0], w[1][1]);
        let pieces = ((seg / PIECE_NM).ceil() as usize).max(1);
        let at = |j: usize| match j {
            0 => w[0],
            j if j == pieces => w[1],
            j => fca::slerp(w[0], w[1], j as f64 / pieces as f64),
        };
        let mut from = at(0);
        for j in 0..pieces {
            let to = at(j + 1);
            if arc_box(from, to, seg / pieces as f64).is_none_or(|b| b.overlaps(bbox)) {
                let lo = if k == 0 && j == 0 {
                    f64::NEG_INFINITY
                } else {
                    acc + seg * j as f64 / pieces as f64 - WINDOW_PAD_NM
                };
                let hi = if k + 1 == legs && j + 1 == pieces {
                    f64::INFINITY
                } else {
                    acc + seg * (j + 1) as f64 / pieces as f64 + WINDOW_PAD_NM
                };
                match windows.last_mut() {
                    Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
                    _ => windows.push((lo, hi)),
                }
            }
            from = to;
        }
        acc += seg;
    }
    windows
}

/// The longest piece of a leg [`along_route_windows`] bounds on its own.
const PIECE_NM: f64 = 20.0;

/// Slack on each window's ends, so a leg boundary rounded differently can never drop a minute.
const WINDOW_PAD_NM: f64 = 0.01;

/// A lat/lon box holding every point of the great circle from `a` to `b` (`seg_nm` long), or `None` when
/// none is worth bounding (near a pole, or half the globe apart in longitude).
///
/// Every point on the arc is within half its length, `r`, of one of its ends, and latitude moves no
/// faster than distance, so the arc stays within `r` of its ends' latitudes: it bulges poleward, so
/// the ends alone are not enough. A great circle meets each meridian once and the shorter arc sweeps at
/// most 180° of longitude, so with its ends less than 180° apart it stays between their longitudes.
/// Both are widened a little for rounding.
fn arc_box(a: [f64; 2], b: [f64; 2], seg_nm: f64) -> Option<Bbox> {
    const EARTH_RADIUS_NM: f64 = 3440.065;
    if (a[1] - b[1]).abs() >= 180.0 {
        return None;
    }
    let r_deg = (seg_nm / EARTH_RADIUS_NM / 2.0).to_degrees() * 1.01 + 1e-9;
    if a[0].abs().max(b[0].abs()) + r_deg >= 89.0 {
        return None;
    }
    Some(Bbox {
        min_lat: a[0].min(b[0]) - r_deg,
        max_lat: a[0].max(b[0]) + r_deg,
        min_lon: a[1].min(b[1]) - 1e-9,
        max_lon: a[1].max(b[1]) + 1e-9,
    })
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
        let loads = sector_loads(&table, &Default::default(), &borrowed, now());
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

    /// A box `[min_lat, max_lat, min_lon, max_lon]`.
    fn bbox([min_lat, max_lat, min_lon, max_lon]: [f64; 4]) -> Bbox {
        Bbox {
            min_lat,
            max_lat,
            min_lon,
            max_lon,
        }
    }

    /// The great circle bulges poleward of its ends, so a box around the ends alone misses it; the
    /// walk skips a leg only when [`arc_box`] misses the ARTCC, so it must hold every point `fca::slerp`
    /// puts on the leg. The legs run up to the 2,000 nm KMIA–KSEA, one is near the Arctic Circle,
    /// one is south of the equator, and one has no length at all.
    #[test]
    fn arc_box_holds_every_point_of_the_great_circle() {
        let legs = [
            ([40.0, -120.0], [40.0, -75.0]),
            ([25.79, -80.29], [47.45, -122.31]),
            ([44.88, -93.22], [33.43, -112.01]),
            ([60.0, -150.0], [62.0, -100.0]),
            ([-33.9, 151.2], [-27.4, 153.1]),
            ([38.85, -77.04], [39.2, -76.4]),
            ([40.64, -73.78], [40.64, -73.78]),
        ];
        for (a, b) in legs {
            let seg = gc_dist(a[0], a[1], b[0], b[1]);
            let arc = arc_box(a, b, seg).expect("a mid-latitude leg is bounded");
            for k in 0..=2000 {
                let p = fca::slerp(a, b, f64::from(k) / 2000.0);
                assert!(
                    arc.contains(p[0], p[1]),
                    "{a:?}->{b:?} at {k}: {p:?} outside {arc:?}"
                );
            }
        }
        // The control: the first leg's midpoint is well north of both its ends.
        let mid = fca::slerp([40.0, -120.0], [40.0, -75.0], 0.5);
        assert!(mid[0] > 42.0, "{mid:?}");
        // Nothing to bound across the antimeridian or near a pole: such a leg is always walked.
        assert!(arc_box([50.0, 179.0], [50.0, -179.0], 77.0).is_none());
        assert!(arc_box([88.5, 0.0], [88.9, 90.0], 60.0).is_none());
    }

    /// Twelve airports and a deterministic spread of traffic between them: airborne anywhere along
    /// the great circle, grounded at the gate, and prefiled, with wheels-up past, near and far.
    fn network() -> (VatsimData, HashMap<String, i64>, AirportDb) {
        let fields = [
            ("KJFK", 40.64, -73.78),
            ("KDCA", 38.85, -77.04),
            ("KORD", 41.98, -87.9),
            ("KATL", 33.64, -84.43),
            ("KLAX", 33.94, -118.41),
            ("KSEA", 47.45, -122.31),
            ("KMIA", 25.79, -80.29),
            ("KDEN", 39.86, -104.67),
            ("KBOS", 42.36, -71.01),
            ("KDFW", 32.9, -97.04),
            ("KMSP", 44.88, -93.22),
            ("KPHX", 33.43, -112.01),
        ];
        let airports: AirportDb = fields
            .iter()
            .map(|(id, lat, lon)| (id.to_string(), Airport::at(*lat, *lon)))
            .collect();
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let pick = |rnd: &mut dyn FnMut() -> f64| {
            fields[(rnd() * fields.len() as f64) as usize % fields.len()]
        };
        let mut data = VatsimData::default();
        let mut wheels_up = HashMap::new();
        for k in 0..150 {
            let dep = pick(&mut rnd);
            let mut arr = pick(&mut rnd);
            while arr.0 == dep.0 {
                arr = pick(&mut rnd);
            }
            let route = if (dep.0, arr.0) == ("KJFK", "KDCA") {
                "RBV WHITE SIE"
            } else {
                ""
            };
            let plan = FlightPlan {
                departure: dep.0.into(),
                arrival: arr.0.into(),
                route: route.into(),
                aircraft_short: ["B738", "A320", "CRJ9", "B77W"][k % 4].into(),
                cruise_tas: format!("{}", 380 + (rnd() * 120.0) as i64),
                altitude: format!("{}", 24_000 + 1_000 * (rnd() * 17.0) as i64),
                ..Default::default()
            };
            let callsign = format!("T{k}");
            let offset_min = -40.0 + rnd() * 340.0;
            let wheels_up_ms = now() + (offset_min * MINUTE_MS as f64) as i64;
            if k % 7 == 0 {
                data.prefiles.push(Prefile {
                    callsign: callsign.clone(),
                    flight_plan: Some(plan),
                    ..Default::default()
                });
                if k % 3 != 0 {
                    wheels_up.insert(callsign, wheels_up_ms);
                }
                continue;
            }
            let grounded = k % 5 == 0;
            let at = fca::slerp(
                [dep.1, dep.2],
                [arr.1, arr.2],
                if grounded { 0.0 } else { rnd() },
            );
            let (lat, lon) = (at[0] + rnd() * 0.4 - 0.2, at[1] + rnd() * 0.4 - 0.2);
            let gs = if grounded {
                0
            } else {
                250 + (rnd() * 250.0) as i64
            };
            if grounded {
                wheels_up.insert(callsign.clone(), wheels_up_ms);
            }
            data.pilots.push(Pilot {
                callsign,
                latitude: lat,
                longitude: lon,
                altitude: if grounded {
                    0
                } else {
                    8_000 + (rnd() * 30_000.0) as i64
                },
                groundspeed: gs,
                heading: fca::bearing_deg([lat, lon], [arr.1, arr.2]) as i64,
                flight_plan: Some(plan),
                ..Default::default()
            });
        }
        (data, wheels_up, airports)
    }

    /// The tracks a box gets, walking the minutes outside it or skipping them, and how many minutes
    /// each resolved.
    fn both_walks(
        data: &VatsimData,
        wheels_up: &HashMap<String, i64>,
        airports: &AirportDb,
        nav: &NavData,
        bbox: Option<Bbox>,
    ) -> [(Vec<OwnedTrack>, usize); 2] {
        let (profiles, winds) = (ProfileTable::default(), Winds::default());
        [true, false].map(|every_minute| {
            let ctx = Ctx {
                nav,
                airports,
                profiles: &profiles,
                winds: &winds,
                now_ms: now(),
                bbox,
                every_minute,
                evaluated: Default::default(),
            };
            let tracks = ctx.project(data, wheels_up, &HashSet::new());
            (tracks, ctx.evaluated.get())
        })
    }

    /// A fix as bits: its time, latitude, longitude and altitude.
    type FixBits = (i64, u64, u64, Option<u64>);

    /// A track as bits, so "the same" means identical, not close.
    fn bits(tracks: &[OwnedTrack]) -> Vec<(String, Population, Vec<FixBits>)> {
        tracks
            .iter()
            .map(|t| {
                let fixes = t
                    .fixes
                    .iter()
                    .map(|f| {
                        (
                            f.t_ms,
                            f.lat.to_bits(),
                            f.lon.to_bits(),
                            f.alt_ft.map(f64::to_bits),
                        )
                    })
                    .collect();
                (t.id.clone(), t.population, fixes)
            })
            .collect()
    }

    /// #725's hot spot: the walk resolved all 361 minutes of every track with `distance_after` and
    /// kept only those inside the ARTCC's box. It now resolves only the minutes on legs that can
    /// reach the box, plus a bisection to find them. This pins it to the every-minute walk, fix for
    /// fix and bit for bit, over 150 flights and twelve boxes: five ARTCC-sized, one a few miles
    /// across, five drawn at random, and no box at all.
    #[test]
    fn skipping_minutes_outside_the_box_changes_no_fix() {
        let (data, wheels_up, airports) = network();
        let nav = NavData::load();
        let mut boxes = vec![
            None,
            Some(bbox([36.5, 40.5, -81.5, -74.5])),
            Some(bbox([39.0, 42.5, -76.5, -71.5])),
            Some(bbox([40.0, 44.0, -91.0, -85.0])),
            Some(bbox([32.0, 37.5, -121.0, -114.0])),
            Some(bbox([41.5, 44.5, -102.0, -95.0])),
            Some(bbox([39.0, 39.3, -77.3, -77.0])),
        ];
        let mut seed = 7u64;
        let mut rnd = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..5 {
            let (lat, lon) = (25.0 + rnd() * 22.0, -124.0 + rnd() * 54.0);
            let (h, w) = (0.2 + rnd() * 6.0, 0.2 + rnd() * 8.0);
            boxes.push(Some(bbox([lat, lat + h, lon, lon + w])));
        }

        let (mut kept, mut every, mut skipping) = (0, 0, 0);
        for b in boxes {
            let [(reference, walked), (windowed, resolved)] =
                both_walks(&data, &wheels_up, &airports, &nav, b);
            assert_eq!(bits(&windowed), bits(&reference), "box {b:?}");
            kept += reference.iter().map(|t| t.fixes.len()).sum::<usize>();
            if b.is_some() {
                every += walked;
                skipping += resolved;
            } else {
                assert_eq!(
                    resolved, walked,
                    "with no box every minute is kept, so none is skipped"
                );
            }
        }
        assert!(kept > 5_000, "the fixtures must keep real fixes: {kept}");
        assert!(
            skipping * 4 < every,
            "minutes resolved: {skipping} skipping, {every} walking every one"
        );
        eprintln!("fixes kept {kept}; minutes resolved {skipping} skipping, {every} walking");
    }
}
