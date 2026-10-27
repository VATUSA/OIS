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
    trajectory::{MAX_PLAUSIBLE_TAS, ias_to_tas, isa_temp_k, mach_to_tas_at},
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
/// At or above this altitude a cruise adds a Mach number beside its TAS.
pub const MACH_BAND_FT: f64 = 28_000.0;
/// A climb or descent speed is an IAS below this altitude.
pub const IAS_TOP_FT: f64 = 26_000.0;
/// A climb or descent speed is a Mach number at or above this altitude. Between [`IAS_TOP_FT`] and
/// here no speed is recorded: the model flies `min(IAS, Mach)` (`trajectory::phase_tas`), so the
/// crossover moves with the type (about FL270 to FL320 for 270–300 KIAS and M0.74–0.84), and a pair
/// near it would record an IAS flown as Mach, or the reverse, slow.
pub const MACH_FLOOR_FT: f64 = 31_000.0;
/// A winds snapshot older than this (s) at the sample time is not used: RAP publishes a cycle every
/// hour and OIS snapshots each one (`jobs.rs` `WINDS_REFRESH_INTERVAL`), so a gap past two hours
/// means missed refreshes.
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

/// Mach number for a true airspeed in air at `temp_k`: the inverse of `trajectory::mach_to_tas_at`,
/// the conversion the model flies a Mach schedule with.
pub fn tas_to_mach(tas_kt: f64, temp_k: f64) -> f64 {
    tas_kt / mach_to_tas_at(1.0, temp_k)
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
        let fpm = (b.altitude - a.altitude) as f64 * 60.0 / dt as f64;
        if fpm.abs() > MAX_PLAUSIBLE_FPM {
            continue;
        }
        let alt = (a.altitude + b.altitude) as f64 / 2.0;
        let air = pair_air(a, b, alt, winds);
        let tas = air.map(|x| x.tas);
        let temp_k = air.map_or_else(|| isa_temp_k(alt), |x| x.temp_k);

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
                    (alt, temp_k),
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
                    (alt, temp_k),
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
                out.push(Field::CruiseMach, tas_to_mach(tas, temp_k));
            }
        }
    }
    out
}

/// A climb or descent speed: Mach at or above [`MACH_FLOOR_FT`], IAS in the low or high band below
/// [`IAS_TOP_FT`], nothing between.
fn push_speed(
    out: &mut FlightSamples,
    (alt, temp_k): (f64, f64),
    tas: f64,
    lo: Field,
    hi: Field,
    mach: Field,
) {
    if alt >= MACH_FLOOR_FT {
        out.push(mach, tas_to_mach(tas, temp_k));
    } else if alt < IAS_TOP_FT {
        out.push(
            if alt < LOW_BAND_FT { lo } else { hi },
            tas_to_ias(tas, alt),
        );
    }
}

/// The air a position pair flew through: its true airspeed and the air temperature (K) the model
/// would fly a Mach number in there.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PairAir {
    tas: f64,
    temp_k: f64,
}

/// Length (nm) of the one-leg path [`pair_air`] samples the field along.
const PROBE_LEG_NM: f64 = 1.0;

/// The pair's true airspeed: the mean reported groundspeed plus the headwind along the track, at the
/// midpoint and mean altitude, read from the field the way the model reads it (`Winds::along` and
/// `RouteWind::air`). The temperature is the field's there, else ISA, as the model falls back
/// (`VerticalProfile`'s Mach cap). None without a fresh winds snapshot, where the field does not
/// cover the point, or when the result is not a believable airspeed.
fn pair_air(
    a: &HistPosition,
    b: &HistPosition,
    alt: f64,
    winds: &WindsTimeline,
) -> Option<PairAir> {
    let w = winds.at(a.ts)?;
    let (pa, pb) = ([a.lat as f64, a.lon as f64], [b.lat as f64, b.lon as f64]);
    // Between two feed ticks a few hundred metres apart the bearing is noise; the reported heading
    // is the better track estimate there.
    let track = if gc_dist(pa[0], pa[1], pb[0], pb[1]) >= 0.5 {
        bearing_deg(pa, pb)
    } else {
        b.heading as f64
    };
    let mid = [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0];
    let ahead = offset(mid, track, PROBE_LEG_NM);
    let (hw, temp) = w.along(&[mid, ahead], PROBE_LEG_NM).air(PROBE_LEG_NM, alt);
    let tas = tas_from_gs((a.groundspeed as f64 + b.groundspeed as f64) / 2.0, hw?);
    (60.0..=MAX_PLAUSIBLE_TAS)
        .contains(&tas)
        .then_some(PairAir {
            tas,
            temp_k: temp.unwrap_or_else(|| isa_temp_k(alt)),
        })
}

/// The point `nm` from `p` along `course_deg`, flat-earth: over one mile the error is far below the
/// field's 1° grid.
fn offset(p: [f64; 2], course_deg: f64, nm: f64) -> [f64; 2] {
    let c = course_deg.to_radians();
    [
        p[0] + nm * c.cos() / 60.0,
        p[1] + nm * c.sin() / (60.0 * p[0].to_radians().cos().max(1e-6)),
    ]
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::feed::{trajectory::mach_to_tas, winds::GRID_MISSING};

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
        // Literal ISA value: 250 KIAS at 10,000 ft is 290.9 KTAS (σ = 0.7385).
        assert!((ias_to_tas(250.0, 10_000.0) - 290.9).abs() < 0.1);
        assert!((tas_to_ias(ias_to_tas(250.0, 10_000.0), 10_000.0) - 250.0).abs() < 1e-9);
        assert!((tas_to_mach(mach_to_tas_at(0.78, 218.81), 218.81) - 0.78).abs() < 1e-12);
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

    /// A RAP-shaped field over 38–42°N, 104–94°W, at every level the same "from" wind vector
    /// `(u, v)` (kt) and temperature `t` (°C, or `GRID_MISSING`).
    fn grid(u: i16, v: i16, t: i16) -> Winds {
        Winds::test_grid(
            (38.0, -104.0),
            (5, 11),
            &[3_000.0, 10_000.0, 18_000.0, 30_000.0, 39_000.0],
            t0(),
            move |_, _, _| (u, v, t),
        )
    }

    /// A position at `lat` (otherwise as [`pos`]).
    fn pos_at(sec: i64, lat: f64, lon: f64, alt: i32, gs: i16) -> HistPosition {
        HistPosition {
            lat: lat as f32,
            ..pos(sec, lon, alt, gs)
        }
    }

    /// Level at FL350 eastbound at `gs`, ten one-minute pairs.
    fn cruise(gs: i16) -> Vec<HistPosition> {
        (0..10)
            .map(|k| pos(k * 60, -100.0 + k as f64 * 0.17, 35_000, gs))
            .collect()
    }

    fn sampled(p: &[HistPosition], w: Winds) -> FlightSamples {
        flight_samples(p, &WindsTimeline(&[(t0() - Duration::minutes(10), w)]))
    }

    /// A Mach at -40 °C (233.15 K; ISA at FL350 is 218.81 K) is the TAS over the speed of sound in
    /// that air: 480 / 595.01 = 0.8067, not ISA's 0.8327. Converting back through the model's own
    /// `mach_to_tas_at` gives the TAS.
    #[test]
    fn a_cruise_mach_in_a_warm_rap_column_inverts_the_models_temperature_conversion() {
        let s = sampled(&cruise(480), grid(0, 0, -40));
        assert!(
            s.0[&Field::CruiseTas]
                .iter()
                .all(|t| (t - 480.0).abs() < 1e-9)
        );
        let m = s.0[&Field::CruiseMach][0];
        assert!((m - 0.8067).abs() < 1e-4, "{m}");
        assert!((mach_to_tas_at(m, 233.15) - 480.0).abs() < 0.01);
    }

    /// A 2,000 fpm climb through FL300–FL320 at 450 kt in a -30 °C column records Mach
    /// 450 / 607.63 = 0.7406 and no high-band IAS.
    #[test]
    fn a_climb_mach_in_a_warm_rap_column_uses_the_column_temperature() {
        let p: Vec<_> = (0..5)
            .map(|k| {
                pos(
                    k * 15,
                    -100.0 + k as f64 * 0.03,
                    33_000 + 500 * k as i32,
                    450,
                )
            })
            .collect();
        let s = sampled(&p, grid(0, 0, -30));
        let m = s.0[&Field::ClimbMach][0];
        assert!((m - 0.7406).abs() < 1e-4, "{m}");
        assert!(!s.0.contains_key(&Field::ClimbIasHi));
    }

    /// Climb pairs either side of the crossover band: an IAS just below FL260, a Mach from FL310, and
    /// between them a rate but no speed.
    #[test]
    fn climb_speeds_skip_the_ias_mach_crossover_band() {
        let at = |alt: i32| {
            let p = vec![
                pos(0, -100.0, alt - 250, 450),
                pos(30, -99.94, alt + 250, 450),
            ];
            sampled(&p, grid(0, 0, -40))
        };
        let below = at(25_900);
        assert!(below.0.contains_key(&Field::ClimbIasHi));
        assert!(!below.0.contains_key(&Field::ClimbMach));
        for alt in [26_000, 28_500, 30_900] {
            let mid = at(alt);
            assert_eq!(mid.0[&Field::ClimbFpmHi], vec![1_000.0], "{alt}");
            assert!(!mid.0.contains_key(&Field::ClimbIasHi), "{alt}");
            assert!(!mid.0.contains_key(&Field::ClimbMach), "{alt}");
        }
        let above = at(31_000);
        assert!(above.0.contains_key(&Field::ClimbMach));
        assert!(!above.0.contains_key(&Field::ClimbIasHi));
    }

    /// Where the field carries no temperature the Mach is taken on ISA, as the model flies it:
    /// 480 / 576.42 = 0.8327 at FL350.
    #[test]
    fn a_mach_sample_without_a_field_temperature_uses_isa() {
        let s = sampled(&cruise(480), grid(0, 0, GRID_MISSING));
        let m = s.0[&Field::CruiseMach][0];
        assert!((m - 0.8327).abs() < 1e-4, "{m}");
    }

    /// The model converts IAS on density altitude alone (`trajectory::ias_to_tas`), so a warm
    /// column leaves an IAS sample where an ISA one puts it: 300 kt TAS at 15,000 ft is 238 KIAS.
    #[test]
    fn an_ias_sample_is_unchanged_by_a_warm_column() {
        let p = vec![pos(0, -100.0, 14_625, 300), pos(30, -99.96, 15_375, 300)];
        let isa = sampled(&p, grid(0, 0, -15)).0[&Field::ClimbIasHi][0];
        let warm = sampled(&p, grid(0, 0, 5)).0[&Field::ClimbIasHi][0];
        assert_eq!(isa, warm);
        assert!((isa - 238.0).abs() < 0.5, "{isa}");
    }

    /// TAS is the mean groundspeed plus the field's headwind along the pair: 50 kt from 090 on an
    /// eastbound pair is a headwind (470), from 270 a tailwind (370).
    #[test]
    fn cruise_tas_adds_the_rap_headwind_along_the_pair() {
        let head = sampled(&cruise(420), grid(50, 0, -50)).0[&Field::CruiseTas].clone();
        assert!(head.iter().all(|t| (t - 470.0).abs() < 0.5), "{head:?}");
        let tail = sampled(&cruise(420), grid(-50, 0, -50)).0[&Field::CruiseTas].clone();
        assert!(tail.iter().all(|t| (t - 370.0).abs() < 0.5), "{tail:?}");
    }

    /// Two fixes 0.2 nm apart are too close for a bearing: the reported heading (090, into a 50 kt
    /// easterly) is the track, though the second fix lies due north.
    #[test]
    fn a_pair_under_half_a_mile_takes_its_track_from_the_reported_heading() {
        let p = vec![
            pos_at(0, 40.0, -100.0, 35_000, 480),
            pos_at(60, 40.0 + 0.2 / 60.0, -100.0, 35_000, 480),
        ];
        let tas = sampled(&p, grid(50, 0, -50)).0[&Field::CruiseTas][0];
        assert!((tas - 530.0).abs() < 0.5, "{tas}");
    }

    /// Two fixes 6 nm apart due north of each other: the bearing (000) is the track, so a 50 kt
    /// easterly is a crosswind and TAS is the groundspeed, whatever the reported heading says.
    #[test]
    fn a_pair_over_half_a_mile_takes_its_track_from_its_own_bearing() {
        let p = vec![
            pos_at(0, 40.0, -100.0, 35_000, 480),
            pos_at(60, 40.1, -100.0, 35_000, 480),
        ];
        let tas = sampled(&p, grid(50, 0, -50)).0[&Field::CruiseTas][0];
        assert!((tas - 480.0).abs() < 0.5, "{tas}");
    }

    /// A snapshot 110 minutes old still gives an airspeed; one 130 minutes old gives none.
    #[test]
    fn winds_110_minutes_old_are_used_and_130_minutes_old_are_not() {
        assert!((6_600..7_800).contains(&WINDS_MAX_AGE_SEC));
        let p = &cruise(420)[..2];
        let at = |min: i64| {
            flight_samples(
                p,
                &WindsTimeline(&[(t0() - Duration::minutes(min), grid(0, 0, -50))]),
            )
        };
        assert_eq!(at(110).0[&Field::CruiseTas], vec![420.0]);
        assert!(at(130).0.is_empty());
    }
}
