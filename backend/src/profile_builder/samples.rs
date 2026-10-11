//! Per-flight performance samples from consecutive stored positions (#828).
//!
//! Each pair of consecutive positions is classified by its vertical rate: climbing, descending, or
//! level at the flight's cruise altitude. A climb or descent pair yields a vertical rate, and with a
//! winds snapshot also an indicated airspeed (or a Mach number high up); a cruise pair yields a true
//! airspeed (and a Mach number high up). Each flight then contributes **one** value per field, the
//! median of its own samples, so a long flight weighs no more than a short one.

use chrono::{DateTime, Utc};

use crate::feed::{
    fca::bearing_deg,
    flow::gc_dist,
    trajectory::{MAX_PLAUSIBLE_TAS, ias_to_tas, mach_to_tas},
    winds::Winds,
};
use crate::repos::profile_history::HistPosition;

/// Below this groundspeed (kt) the aircraft is on the ground. Matches the airborne test in
/// [`crate::feed::predict::arrival_eta`].
pub const AIRBORNE_GS_KT: i16 = 50;
/// A climb or descent pair must be at most this far apart (s). Positions keep full resolution for 7
/// days and one in four (about a minute apart) for the next 7 (`jobs.rs` `COMPACTION_TIERS`); the
/// ~4- and ~16-minute spacing older than that can't time a climb or descent.
pub const MAX_RATE_GAP_SEC: i64 = 90;
/// A cruise pair may be this far apart (s): the speed comes from the groundspeed the feed reported
/// at each end, not from the distance between them, so thinned history still serves cruise.
pub const MAX_CRUISE_GAP_SEC: i64 = 20 * 60;
/// Pairs closer than this (s) are duplicates of one feed tick.
const MIN_GAP_SEC: i64 = 5;
/// A pair climbing at least this fast (fpm) is a climb sample; descending at least this fast, a
/// descent sample. Slower than this is a level-off or noise.
pub const MIN_VERTICAL_FPM: f64 = 300.0;
/// A pair changing altitude slower than this (fpm) is level.
pub const LEVEL_MAX_FPM: f64 = 100.0;
/// A level pair within this many feet of the flight's highest altitude is at cruise. Matches
/// `trajectory::ANCHOR_ALT_TOLERANCE_FT`, the model's own "established at cruise" band.
pub const CRUISE_BAND_FT: f64 = 300.0;
/// Faster than this (fpm) is a teleport or a sim glitch, not flying.
const MAX_PLAUSIBLE_FPM: f64 = 8_000.0;
/// The model's low/high speed-schedule boundary (`AircraftProfile::climb_ias_lo` is "below 10,000 ft").
pub const LOW_BAND_FT: f64 = 10_000.0;
/// At or above this altitude a climb, cruise or descent speed is recorded as a Mach number instead
/// of an indicated airspeed: the IAS/Mach crossover of a jet sits around FL280–FL300.
pub const MACH_BAND_FT: f64 = 28_000.0;
/// A winds snapshot older than this (s) at the sample time is not used: AWC issues the tables about
/// four times a day and OIS snapshots them hourly, so a gap past two hours means a missed refresh.
pub const WINDS_MAX_AGE_SEC: i64 = 2 * 60 * 60;

/// One performance field of [`crate::feed::trajectory::AircraftProfile`] a flight can measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    ClimbIasLo,
    ClimbIasHi,
    ClimbMach,
    ClimbFpmLo,
    ClimbFpmHi,
    CruiseTas,
    CruiseMach,
    DescMach,
    DescIasHi,
    DescIasLo,
    DescFpm,
}

impl Field {
    pub const ALL: [Field; 11] = [
        Field::ClimbIasLo,
        Field::ClimbIasHi,
        Field::ClimbMach,
        Field::ClimbFpmLo,
        Field::ClimbFpmHi,
        Field::CruiseTas,
        Field::CruiseMach,
        Field::DescMach,
        Field::DescIasHi,
        Field::DescIasLo,
        Field::DescFpm,
    ];

    /// The `AircraftProfile` / `flow.aircraft_profile` column name.
    pub fn name(self) -> &'static str {
        match self {
            Field::ClimbIasLo => "climb_ias_lo",
            Field::ClimbIasHi => "climb_ias_hi",
            Field::ClimbMach => "climb_mach",
            Field::ClimbFpmLo => "climb_fpm_lo",
            Field::ClimbFpmHi => "climb_fpm_hi",
            Field::CruiseTas => "cruise_tas",
            Field::CruiseMach => "cruise_mach",
            Field::DescMach => "desc_mach",
            Field::DescIasHi => "desc_ias_hi",
            Field::DescIasLo => "desc_ias_lo",
            Field::DescFpm => "desc_fpm",
        }
    }

    pub fn from_name(name: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.name() == name)
    }

    pub fn is_mach(self) -> bool {
        matches!(self, Field::ClimbMach | Field::CruiseMach | Field::DescMach)
    }
}

/// The winds snapshots around one flight, oldest first, as `repos::profile_history::winds_covering`
/// returns them.
pub struct WindsTimeline<'a>(pub &'a [(DateTime<Utc>, Winds)]);

impl WindsTimeline<'_> {
    /// The snapshot in force at `at` (the newest at or before it), unless it is older than
    /// [`WINDS_MAX_AGE_SEC`].
    pub fn at(&self, at: DateTime<Utc>) -> Option<&Winds> {
        let i = self.0.partition_point(|(ts, _)| *ts <= at);
        let (ts, w) = self.0.get(i.checked_sub(1)?)?;
        ((at - *ts).num_seconds() <= WINDS_MAX_AGE_SEC).then_some(w)
    }
}

/// True airspeed from a groundspeed and the headwind along the track: the inverse of
/// [`crate::feed::trajectory::effective_gs`] (`gs = tas − headwind`), so a measured TAS fed back
/// through the model reproduces the groundspeed it came from.
pub fn tas_from_gs(gs_kt: f64, headwind_kt: f64) -> f64 {
    gs_kt + headwind_kt
}

/// Indicated airspeed for a true airspeed at altitude: the inverse of `trajectory::ias_to_tas`,
/// which is linear in IAS.
pub fn tas_to_ias(tas_kt: f64, alt_ft: f64) -> f64 {
    tas_kt / ias_to_tas(1.0, alt_ft)
}

/// Mach number for a true airspeed at altitude: the inverse of `trajectory::mach_to_tas`.
pub fn tas_to_mach(tas_kt: f64, alt_ft: f64) -> f64 {
    tas_kt / mach_to_tas(1.0, alt_ft)
}

/// Every sample one flight yields, per field, in position order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FlightSamples(pub std::collections::BTreeMap<Field, Vec<f64>>);

impl FlightSamples {
    fn push(&mut self, f: Field, v: f64) {
        self.0.entry(f).or_default().push(v);
    }

    /// The flight's one value per field: the median of its own samples.
    pub fn per_flight_medians(&self) -> Vec<(Field, f64)> {
        self.0
            .iter()
            .filter_map(|(f, v)| super::stats::median(v).map(|m| (*f, m)))
            .collect()
    }
}

/// Classify one flight's consecutive positions (oldest first) and collect its samples.
pub fn flight_samples(positions: &[HistPosition], winds: &WindsTimeline) -> FlightSamples {
    let mut out = FlightSamples::default();
    let Some(max_alt) = positions
        .iter()
        .filter(|p| p.groundspeed >= AIRBORNE_GS_KT)
        .map(|p| p.altitude)
        .max()
    else {
        return out;
    };
    let max_alt = max_alt as f64;

    for w in positions.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if a.groundspeed < AIRBORNE_GS_KT || b.groundspeed < AIRBORNE_GS_KT {
            continue;
        }
        let dt = (b.ts - a.ts).num_seconds();
        if dt < MIN_GAP_SEC {
            continue;
        }
        let fpm = (b.altitude - a.altitude) as f64 / dt as f64 * 60.0;
        if fpm.abs() > MAX_PLAUSIBLE_FPM {
            continue;
        }
        let alt = (a.altitude + b.altitude) as f64 / 2.0;
        let tas = pair_tas(a, b, alt, winds);

        if fpm >= MIN_VERTICAL_FPM && dt <= MAX_RATE_GAP_SEC {
            out.push(
                if alt < LOW_BAND_FT {
                    Field::ClimbFpmLo
                } else {
                    Field::ClimbFpmHi
                },
                fpm,
            );
            if let Some(tas) = tas {
                push_speed(
                    &mut out,
                    alt,
                    tas,
                    Field::ClimbIasLo,
                    Field::ClimbIasHi,
                    Field::ClimbMach,
                );
            }
        } else if fpm <= -MIN_VERTICAL_FPM && dt <= MAX_RATE_GAP_SEC {
            out.push(Field::DescFpm, -fpm);
            if let Some(tas) = tas {
                push_speed(
                    &mut out,
                    alt,
                    tas,
                    Field::DescIasLo,
                    Field::DescIasHi,
                    Field::DescMach,
                );
            }
        } else if fpm.abs() <= LEVEL_MAX_FPM
            && dt <= MAX_CRUISE_GAP_SEC
            && (max_alt - a.altitude as f64) <= CRUISE_BAND_FT
            && (max_alt - b.altitude as f64) <= CRUISE_BAND_FT
            && let Some(tas) = tas
        {
            out.push(Field::CruiseTas, tas);
            if alt >= MACH_BAND_FT {
                out.push(Field::CruiseMach, tas_to_mach(tas, alt));
            }
        }
    }
    out
}

/// A climb or descent speed: Mach at or above [`MACH_BAND_FT`], otherwise IAS in the low or high band.
fn push_speed(out: &mut FlightSamples, alt: f64, tas: f64, lo: Field, hi: Field, mach: Field) {
    if alt >= MACH_BAND_FT {
        out.push(mach, tas_to_mach(tas, alt));
    } else {
        out.push(
            if alt < LOW_BAND_FT { lo } else { hi },
            tas_to_ias(tas, alt),
        );
    }
}

/// The pair's true airspeed: the mean reported groundspeed plus the headwind along the track between
/// the two points, at their midpoint and mean altitude. None without a fresh winds snapshot, when no
/// station covers the point, or when the result is not a believable airspeed.
fn pair_tas(a: &HistPosition, b: &HistPosition, alt: f64, winds: &WindsTimeline) -> Option<f64> {
    let w = winds.at(a.ts)?;
    let (pa, pb) = ([a.lat as f64, a.lon as f64], [b.lat as f64, b.lon as f64]);
    // Between two feed ticks a few hundred metres apart the bearing is noise; the reported heading
    // is the better track estimate there.
    let track = if gc_dist(pa[0], pa[1], pb[0], pb[1]) >= 0.5 {
        bearing_deg(pa, pb)
    } else {
        b.heading as f64
    };
    let mid = ((pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0);
    let hw = w.point_headwind(mid.0, mid.1, track, alt)?;
    let gs = (a.groundspeed as f64 + b.groundspeed as f64) / 2.0;
    let tas = tas_from_gs(gs, hw);
    (60.0..=MAX_PLAUSIBLE_TAS).contains(&tas).then_some(tas)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap()
    }

    /// A position `sec` seconds after `t0`, flying due east along 40°N.
    fn pos(sec: i64, lon: f64, alt: i32, gs: i16) -> HistPosition {
        HistPosition {
            ts: t0() + Duration::seconds(sec),
            lat: 40.0,
            lon: lon as f32,
            altitude: alt,
            groundspeed: gs,
            heading: 90,
        }
    }

    /// A single station at the flight's latitude with the same wind at every level.
    fn winds(dir: f64, spd: f64) -> Winds {
        serde_json::from_value(serde_json::json!({
            "stations": [{
                "lat": 40.0, "lon": -100.0,
                "levels": [[3000, {"dir": dir, "spd": spd}], [39000, {"dir": dir, "spd": spd}]]
            }]
        }))
        .unwrap()
    }

    #[test]
    fn conversions_invert_the_model() {
        // Literal ISA values: 250 KIAS at 10,000 ft is about 288 KTAS; M0.78 at FL350 about 450 KTAS.
        assert!((ias_to_tas(250.0, 10_000.0) - 288.0).abs() < 1.5);
        assert!((tas_to_ias(ias_to_tas(250.0, 10_000.0), 10_000.0) - 250.0).abs() < 1e-9);
        assert!((tas_to_mach(mach_to_tas(0.78, 35_000.0), 35_000.0) - 0.78).abs() < 1e-12);
        assert_eq!(tas_from_gs(400.0, 50.0), 450.0);
    }

    #[test]
    fn a_climb_records_its_rate_per_band() {
        // 2,000 fpm below 10,000 ft, 1,500 fpm above, 15 s ticks.
        let mut p = vec![];
        let (mut alt, mut lon) = (2_000, -100.0);
        for k in 0..40 {
            p.push(pos(k * 15, lon, alt, 280));
            alt += if alt < 10_000 { 500 } else { 375 };
            lon += 0.02;
        }
        let s = flight_samples(&p, &WindsTimeline(&[]));
        let m: std::collections::BTreeMap<_, _> = s.per_flight_medians().into_iter().collect();
        assert_eq!(m[&Field::ClimbFpmLo], 2_000.0);
        assert_eq!(m[&Field::ClimbFpmHi], 1_500.0);
        assert!(
            !m.contains_key(&Field::ClimbIasLo),
            "no winds snapshot, no airspeed"
        );
        assert!(!m.contains_key(&Field::DescFpm));
    }

    #[test]
    fn cruise_tas_adds_the_headwind_back() {
        // Level at FL350 eastbound into a 50 kt easterly (a headwind), 420 kt over the ground.
        let snap = [(t0() - Duration::minutes(30), winds(90.0, 50.0))];
        let p: Vec<_> = (0..10)
            .map(|k| pos(k * 60, -100.0 + k as f64 * 0.12, 35_000, 420))
            .collect();
        let s = flight_samples(&p, &WindsTimeline(&snap));
        let tas = &s.0[&Field::CruiseTas];
        assert_eq!(tas.len(), 9);
        assert!(tas.iter().all(|t| (t - 470.0).abs() < 0.5), "{tas:?}");
        let mach = s.0[&Field::CruiseMach][0];
        assert!((mach - 470.0 / mach_to_tas(1.0, 35_000.0)).abs() < 1e-3);
    }

    #[test]
    fn a_stale_winds_snapshot_gives_no_airspeed() {
        let snap = [(
            t0() - Duration::seconds(WINDS_MAX_AGE_SEC + 60),
            winds(90.0, 50.0),
        )];
        let p: Vec<_> = (0..5)
            .map(|k| pos(k * 60, -100.0 + k as f64 * 0.12, 35_000, 420))
            .collect();
        assert!(flight_samples(&p, &WindsTimeline(&snap)).0.is_empty());
        // Positive control: the same snapshot just inside the limit is used.
        let fresh = [(
            t0() - Duration::seconds(WINDS_MAX_AGE_SEC - 60),
            winds(90.0, 50.0),
        )];
        assert!(!flight_samples(&p, &WindsTimeline(&fresh)).0.is_empty());
    }

    #[test]
    fn widely_spaced_positions_feed_cruise_but_not_rates() {
        // Four minutes apart (history 14–21 days old): a climb is not timed, cruise still counts.
        let snap = [(t0() - Duration::minutes(10), winds(270.0, 0.0))];
        let mut p = vec![pos(0, -100.0, 20_000, 400), pos(240, -99.6, 30_000, 420)];
        p.push(pos(480, -99.2, 30_000, 430));
        p.push(pos(720, -98.8, 30_000, 430));
        let s = flight_samples(&p, &WindsTimeline(&snap));
        assert!(!s.0.contains_key(&Field::ClimbFpmHi));
        assert_eq!(s.0[&Field::CruiseTas].len(), 2);
    }

    #[test]
    fn a_descent_and_a_level_off_below_cruise_are_told_apart() {
        let mut p = vec![];
        let mut alt = 35_000;
        for k in 0..20 {
            p.push(pos(k * 30, -100.0 + k as f64 * 0.05, alt, 400));
            alt -= 900; // 1,800 fpm
        }
        // A level segment at 17,000 ft is not cruise: the flight topped out at 35,000.
        for k in 20..25 {
            p.push(pos(k * 30, -100.0 + k as f64 * 0.05, 17_000, 300));
        }
        let s = flight_samples(&p, &WindsTimeline(&[]));
        assert_eq!(s.0[&Field::DescFpm], vec![1_800.0; 20]);
        assert!(!s.0.contains_key(&Field::CruiseTas));
    }

    #[test]
    fn ground_positions_and_glitches_are_ignored() {
        let p = vec![
            pos(0, -100.0, 1_000, 10),
            pos(15, -100.0, 1_000, 20),
            pos(30, -100.0, 1_000, 140),
            pos(45, -99.99, 6_000, 160), // 20,000 fpm: a slew, not a climb
            pos(60, -99.98, 6_500, 170),
        ];
        let s = flight_samples(&p, &WindsTimeline(&[]));
        assert_eq!(s.0[&Field::ClimbFpmLo], vec![2_000.0]);
    }
}
