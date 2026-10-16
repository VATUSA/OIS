//! Shared trajectory / ETA model — a per-aircraft vertical-profile integrator with winds-aloft
//! correction. This is the single predictor used by every surface that estimates where connected
//! aircraft will be at a given time (FCA metering, airport-flow demand, runway ETE), so those
//! surfaces stay consistent. The crossing/arrival geometry ports from vatflow's `fca-metering.js`;
//! the performance model is OIS's own configurable [`AircraftProfile`] / [`VerticalProfile`].
//!
//! A profile integrates a climb (per-band IAS schedule + rates), a cruise (TAS or Mach), and a
//! descent to a real top-of-descent, with a Mach↔TAS crossover. Wind and temperature come from a
//! [`RouteWind`] sampled at each point's distance and altitude ([`crate::feed::winds`]); the Mach
//! portion uses the air temperature there, and ISA where the field has none. It fails safe to still
//! air when winds are unavailable.

/// Upper bound on a *believable* filed cruise TAS (kt). Pilots sometimes file garbage (e.g. `4800`),
/// and since the ETA clamps groundspeed only *relative* to TAS, an absurd value would collapse the
/// transit time and shoot the aircraft to the front of the sequence. Anything above this is treated
/// as unfiled and replaced with the altitude-based default. Comfortably above any real airliner.
pub const MAX_PLAUSIBLE_TAS: f64 = 700.0;

/// Parse a filed cruise-altitude string (`"350"`, `"FL350"`, `"35000"`) to feet. Values
/// ≤ 600 are treated as flight levels (×100); junk falls back to 35,000 ft.
pub fn parse_alt_ft(s: &str) -> f64 {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 35000.0;
    }
    let mut n: f64 = digits.parse().unwrap_or(35000.0);
    if n <= 600.0 {
        n *= 100.0;
    }
    if n < 1000.0 { 35000.0 } else { n }
}

/// Filed TAS (kt) with a sane fallback estimated from cruise altitude. A filed value outside the
/// believable band (too slow, or garbage like `4800`) is discarded for the altitude-based default.
pub fn tas_or_default(tas: f64, cruise_alt_ft: f64) -> f64 {
    if (60.0..=MAX_PLAUSIBLE_TAS).contains(&tas) {
        return tas;
    }
    if cruise_alt_ft >= 28000.0 {
        440.0
    } else if cruise_alt_ft >= 15000.0 {
        300.0
    } else {
        170.0
    }
}

/// Groundspeed from TAS and headwind (`+` = headwind, `-` = tailwind), clamped to a sane
/// band so a bad wind reading can't produce absurd speeds. Still air when `headwind` is None.
pub fn effective_gs(tas: f64, headwind: Option<f64>) -> f64 {
    match headwind {
        None => tas,
        Some(hw) => {
            // Order the bounds so a nonsensical negative TAS can't invert the clamp range.
            let (lo, hi) = if tas >= 0.0 {
                (tas * 0.4, tas * 1.6)
            } else {
                (tas * 1.6, tas * 0.4)
            };
            (tas - hw).clamp(lo, hi)
        }
    }
}

/// The cruise true airspeed to use for a flight: **the profile caps the filing**. When the profile
/// sets a `cruise_tas`, a believable filed TAS is capped to it (and the profile's value is used
/// outright when the filing is missing/garbage) — so a mis-filed 400 kt C172 is still metered at
/// its real ~110 kt. With no profile cruise_tas, we fall back to the legacy filed-or-altitude TAS.
pub fn capped_cruise_tas(filed_tas: f64, cruise_alt_ft: f64, profile: &AircraftProfile) -> f64 {
    let believable = (60.0..=MAX_PLAUSIBLE_TAS).contains(&filed_tas);
    match profile.cruise_tas {
        Some(cap) if believable => filed_tas.min(cap),
        Some(cap) => cap,
        None => tas_or_default(filed_tas, cruise_alt_ft),
    }
}

// ---------------------------------------------------------------------------
// Vertical-profile model (configurable per-aircraft performance).
//
// The legacy `profile_transit_sec` above times a climb-to-cruise only and treats its speed
// constants as groundspeeds. The model below is the richer, configurable replacement: a
// per-aircraft `AircraftProfile` (climb / cruise / descent speed schedules with an ISA
// Mach↔TAS crossover, climb & descent rates, and a service ceiling) integrated over a route
// with a real top-of-descent. Speed-schedule values are indicated airspeeds (KIAS) converted
// to true airspeed by the standard atmosphere, so a profile's numbers mean what they do on a
// SimBrief airframe. Phase C migrates the callers from the legacy fn to this model.
// ---------------------------------------------------------------------------

/// Standard-atmosphere constants (ISA, sea level).
const ISA_T0_K: f64 = 288.15; // sea-level temperature, K
const ISA_LAPSE_K_PER_FT: f64 = 0.0019812; // 1.98 K per 1000 ft, troposphere
const ISA_TROPOPAUSE_FT: f64 = 36089.0;
const ISA_TROPOPAUSE_T_K: f64 = 216.65; // isothermal above the tropopause
const SPEED_OF_SOUND_SL_KT: f64 = 661.4788; // a0 at ISA sea level, knots

/// ISA temperature (K) at a pressure altitude.
fn isa_temp_k(alt_ft: f64) -> f64 {
    if alt_ft <= ISA_TROPOPAUSE_FT {
        ISA_T0_K - ISA_LAPSE_K_PER_FT * alt_ft
    } else {
        ISA_TROPOPAUSE_T_K
    }
}

/// ISA density ratio σ = ρ/ρ₀ at a pressure altitude.
fn isa_density_ratio(alt_ft: f64) -> f64 {
    if alt_ft <= ISA_TROPOPAUSE_FT {
        // σ = (T/T0)^(g/(L·R) − 1) = θ^4.2558797 in the troposphere.
        (isa_temp_k(alt_ft) / ISA_T0_K).powf(4.2558797)
    } else {
        // Isothermal layer: σ at 36089 ft × exp(−(h−h_trop)/H), scale height ≈ 20805 ft.
        let sigma_trop = (ISA_TROPOPAUSE_T_K / ISA_T0_K).powf(4.2558797);
        sigma_trop * (-(alt_ft - ISA_TROPOPAUSE_FT) / 20805.7).exp()
    }
}

/// True airspeed (kt) for an indicated airspeed at altitude: TAS = IAS / √σ. A first-order
/// (incompressible) correction — good enough for ETA modelling across the flight envelope.
pub fn ias_to_tas(ias_kt: f64, alt_ft: f64) -> f64 {
    ias_kt / isa_density_ratio(alt_ft.max(0.0)).max(1e-3).sqrt()
}

/// True airspeed (kt) for a Mach number at altitude in the standard atmosphere: TAS = M · a,
/// a = a₀·√(T/T₀). The model itself uses [`mach_to_tas_at`] with the field's temperature.
#[cfg(test)]
pub fn mach_to_tas(mach: f64, alt_ft: f64) -> f64 {
    mach_to_tas_at(mach, isa_temp_k(alt_ft.max(0.0)))
}

/// True airspeed (kt) for a Mach number in air at `temp_k`: the speed of sound scales with √T, so
/// the same Mach is faster in warm air than in ISA.
pub fn mach_to_tas_at(mach: f64, temp_k: f64) -> f64 {
    mach * SPEED_OF_SOUND_SL_KT * (temp_k.max(1.0) / ISA_T0_K).sqrt()
}

/// One sample of the wind along a route: the headwind (kt, `+` = headwind) along the route's course
/// and the air temperature (K) where known, at each altitude (ft), in increasing altitude.
#[derive(Debug, Clone, PartialEq)]
pub struct WindKnot {
    /// Distance to destination (nm) of this sample.
    pub d_nm: f64,
    /// `(altitude_ft, headwind_kt, temp_k)`, in increasing altitude.
    pub levels: Vec<(f64, f64, Option<f64>)>,
}

/// The wind a profile flies through, by distance to destination and altitude (#836). Built per
/// route by [`crate::feed::winds::Winds::along`], so each point of the profile reads the wind at its
/// own position and altitude rather than one mean for the whole flight. Between knots and levels
/// the values interpolate linearly, except next to a gap ([`RouteWind::new`]); outside them they
/// hold the nearest. Empty is still air.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteWind {
    knots: Vec<WindKnot>,
    /// No knot carries a level: still air everywhere. Kept so a query need not scan the knots.
    still: bool,
}

impl Default for RouteWind {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl RouteWind {
    /// No wind and no temperature: the profile flies still air on ISA.
    pub fn still() -> Self {
        Self::default()
    }

    /// Knots in any order. A knot with no levels is a gap: the field does not cover that point.
    /// Next to a covered knot the covered value holds up to the gap; between two gaps the air is
    /// still. With samples at most [`crate::feed::winds`]'s spacing apart, a covered wind reaches at
    /// most one sample past the edge of the field.
    pub fn new(mut knots: Vec<WindKnot>) -> Self {
        knots.sort_by(|a, b| a.d_nm.total_cmp(&b.d_nm));
        for k in &mut knots {
            k.levels.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        let still = knots.iter().all(|k| k.levels.is_empty());
        Self { knots, still }
    }

    /// The same headwind everywhere, at every altitude, with no temperature: one route-mean value,
    /// as the model applied before #836, for tests that compare against it.
    #[cfg(test)]
    pub fn uniform(headwind_kt: f64) -> Self {
        Self::new(vec![WindKnot {
            d_nm: 0.0,
            levels: vec![(0.0, headwind_kt, None)],
        }])
    }

    pub fn is_still(&self) -> bool {
        self.still
    }

    /// The knots' distances to destination, in increasing order: what a test of how a route was
    /// sampled can see.
    #[cfg(test)]
    pub(crate) fn knot_distances(&self) -> Vec<f64> {
        self.knots.iter().map(|k| k.d_nm).collect()
    }

    /// Headwind (kt) at distance-to-destination `d_nm` and `alt_ft`; None in still air.
    pub fn headwind(&self, d_nm: f64, alt_ft: f64) -> Option<f64> {
        self.air(d_nm, alt_ft).0
    }

    /// Air temperature (K) at distance-to-destination `d_nm` and `alt_ft`; None where the field
    /// carries no temperature.
    pub fn temp_k(&self, d_nm: f64, alt_ft: f64) -> Option<f64> {
        self.air(d_nm, alt_ft).1
    }

    /// Headwind and temperature at `(d, alt)` in one lookup: linear between the bracketing knots,
    /// held at the nearest outside them, and a knot missing a field defers to the other knot.
    pub fn air(&self, d_nm: f64, alt_ft: f64) -> (Option<f64>, Option<f64>) {
        let i = self.knots.partition_point(|k| k.d_nm < d_nm);
        match (i.checked_sub(1).map(|j| &self.knots[j]), self.knots.get(i)) {
            (Some(k0), Some(k1)) => {
                let (a0, a1) = (
                    by_altitude(&k0.levels, alt_ft),
                    by_altitude(&k1.levels, alt_ft),
                );
                let span = k1.d_nm - k0.d_nm;
                let t = if span > 1e-9 {
                    (d_nm - k0.d_nm) / span
                } else {
                    0.0
                };
                (between(a0.0, a1.0, t), between(a0.1, a1.1, t))
            }
            (Some(k), None) | (None, Some(k)) => by_altitude(&k.levels, alt_ft),
            (None, None) => (None, None),
        }
    }
}

/// How far past a gap knot the profile takes its still-air sample.
const GAP_EDGE_NM: f64 = 1e-3;

/// The distances at which a profile samples one wind knot. The wind and temperature step at a gap
/// knot (the covered value holds up to it, [`RouteWind::new`]), so a gap knot also gets a sample just
/// past it; without it the samples would ramp the covered value across the next span. A distance
/// exactly at a knot reads the span that ends there, so no sample is needed before it.
fn knot_sample_distances(k: &WindKnot) -> impl Iterator<Item = f64> {
    std::iter::once(k.d_nm).chain(k.levels.is_empty().then_some(k.d_nm + GAP_EDGE_NM))
}

/// `v0` to `v1` at fraction `t`; with one side missing, the other.
fn between(v0: Option<f64>, v1: Option<f64>, t: f64) -> Option<f64> {
    match (v0, v1) {
        (Some(a), Some(b)) => Some(a + t * (b - a)),
        (a, b) => a.or(b),
    }
}

/// The headwind and the temperature of one knot at `alt_ft`, each linear between the bracketing
/// levels that carry it and held at the nearest outside them. `levels` is in increasing altitude.
fn by_altitude(levels: &[(f64, f64, Option<f64>)], alt_ft: f64) -> (Option<f64>, Option<f64>) {
    let (mut hw_below, mut hw_above) = (None, None);
    let (mut t_below, mut t_above) = (None, None);
    for &(a, hw, t) in levels {
        if a <= alt_ft {
            hw_below = Some((a, hw));
        } else if hw_above.is_none() {
            hw_above = Some((a, hw));
        }
        if let Some(t) = t {
            if a <= alt_ft {
                t_below = Some((a, t));
            } else if t_above.is_none() {
                t_above = Some((a, t));
            }
        }
    }
    (
        lerp_levels(hw_below, hw_above, alt_ft),
        lerp_levels(t_below, t_above, alt_ft),
    )
}

fn lerp_levels(below: Option<(f64, f64)>, above: Option<(f64, f64)>, alt_ft: f64) -> Option<f64> {
    match (below, above) {
        (Some((a0, v0)), Some((a1, v1))) => Some(v0 + (alt_ft - a0) / (a1 - a0) * (v1 - v0)),
        (Some((_, v)), None) | (None, Some((_, v))) => Some(v),
        (None, None) => None,
    }
}

/// A configurable per-aircraft performance profile. `Default` reproduces the legacy
/// climb/cruise numbers (250/290 kt, 2000/1500 fpm, no Mach) plus a plain jet descent, so a
/// caller with no matching profile behaves like the old model (now with a descent phase).
#[derive(Debug, Clone, PartialEq)]
pub struct AircraftProfile {
    // Climb speed schedule (KIAS) + rates (fpm).
    pub climb_ias_lo: f64, // below 10,000 ft
    pub climb_ias_hi: f64, // 10,000 ft to the Mach crossover
    pub climb_mach: Option<f64>,
    pub climb_fpm_lo: f64, // below 10,000 ft
    pub climb_fpm_hi: f64, // above 10,000 ft
    // Cruise. `cruise_tas` is a ceiling on the (capped) filed TAS; `cruise_mach` governs high up.
    pub cruise_tas: Option<f64>,
    pub cruise_mach: Option<f64>,
    pub service_ceiling_ft: f64,
    // Descent speed schedule (KIAS) + rate (fpm).
    pub desc_mach: Option<f64>,
    pub desc_ias_hi: f64, // Mach crossover down to 10,000 ft
    pub desc_ias_lo: f64, // below 10,000 ft
    pub desc_fpm: f64,
}

impl Default for AircraftProfile {
    fn default() -> Self {
        Self {
            climb_ias_lo: 250.0,
            climb_ias_hi: 290.0,
            climb_mach: None,
            climb_fpm_lo: 2000.0,
            climb_fpm_hi: 1500.0,
            cruise_tas: None,
            cruise_mach: None,
            service_ceiling_ft: 45000.0,
            desc_mach: None,
            desc_ias_hi: 290.0,
            desc_ias_lo: 250.0,
            desc_fpm: 1800.0,
        }
    }
}

/// A resolved set of aircraft profiles with the three-tier fallback used to pick performance for
/// a flight: exact ICAO type → wake class (`L/M/H/J`) → the global default. Built from the DB by
/// [`crate::repos::aircraft_profiles`] and cached in `AppState`; `Default` (legacy numbers, no
/// type/wake overrides) is the safe fallback before the table has loaded.
#[derive(Debug, Clone, Default)]
pub struct ProfileTable {
    default: AircraftProfile,
    by_wake: std::collections::HashMap<String, AircraftProfile>,
    by_type: std::collections::HashMap<String, AircraftProfile>,
}

impl ProfileTable {
    pub fn new(
        default: AircraftProfile,
        by_wake: std::collections::HashMap<String, AircraftProfile>,
        by_type: std::collections::HashMap<String, AircraftProfile>,
    ) -> Self {
        Self {
            default,
            by_wake,
            by_type,
        }
    }

    /// The performance profile for an aircraft: exact type → wake class → global default.
    pub fn resolve(&self, ty: &str, wake: &str) -> &AircraftProfile {
        self.by_type
            .get(&ty.to_ascii_uppercase())
            .or_else(|| self.by_wake.get(&wake.to_ascii_uppercase()))
            .unwrap_or(&self.default)
    }

    /// A label for which tier `resolve` matched, for the debug view: `type:<T>`, `wake:<W>`, or
    /// `default`.
    pub fn resolve_label(&self, ty: &str, wake: &str) -> String {
        let ty = ty.to_ascii_uppercase();
        let wake = wake.to_ascii_uppercase();
        if self.by_type.contains_key(&ty) {
            format!("type:{ty}")
        } else if self.by_wake.contains_key(&wake) {
            format!("wake:{wake}")
        } else {
            "default".to_string()
        }
    }
}

/// The governing true airspeed for a climb/descent phase at `alt`: the IAS-schedule TAS,
/// held down to the Mach number's TAS when one is set (the SimBrief crossover — Mach wins high,
/// IAS wins low). `ias` is the schedule speed for the altitude band; `mach` the phase Mach, flown in
/// air at `temp_k`.
fn phase_tas(ias_lo: f64, ias_hi: f64, mach: Option<f64>, alt_ft: f64, temp_k: f64) -> f64 {
    let ias = if alt_ft < 10_000.0 { ias_lo } else { ias_hi };
    let tas_ias = ias_to_tas(ias, alt_ft);
    match mach {
        Some(m) => tas_ias.min(mach_to_tas_at(m, temp_k)),
        None => tas_ias,
    }
}

/// Groundspeed floor so a stalled step can't divide by ~zero.
const GS_FLOOR_KT: f64 = 60.0;
/// Vertical integration step (ft) for building the climb/descent altitude-vs-distance curves.
const VERT_STEP_FT: f64 = 500.0;
/// Horizontal integration step (nm) for timing a distance span across the profile.
const HORIZ_STEP_NM: f64 = 3.0;
/// Groundspeed anchoring (#313) trusts the observed groundspeed only for an aircraft established at
/// cruise: at least this fast…
const ANCHOR_MIN_GS_KT: f64 = 250.0;
/// …and within this many feet of the profile's cruise altitude. A climbing departure is not anchored.
const ANCHOR_ALT_TOLERANCE_FT: f64 = 300.0;
/// The anchoring bias is clamped to ±this fraction, so one noisy `gs` sample can't throw the
/// trajectory.
const ANCHOR_MAX_BIAS: f64 = 0.15;

/// A resolved vertical flight profile, anchored at the aircraft's current state and running
/// forward to the destination. Built once per aircraft, then queried for the altitude at, and
/// the time to reach, any point measured as distance-to-destination.
///
/// Distances are **nm-to-destination** (`d`): `d = 0` is the field, increasing back along the
/// route. The aircraft is at `(start_alt, start_d)`. Ahead of it: climb from `start_alt` to the
/// achieved cruise altitude, cruise, then descend from top-of-descent to the field.
pub struct VerticalProfile {
    /// The wind and temperature along the route, read at each point's `d` and altitude.
    wind: RouteWind,
    /// Multiplier on every predicted groundspeed; `1.0` unless [`Self::anchor_to_observed_gs`] set it.
    gs_bias: f64,
    /// The aircraft's starting altitude and distance, and the cruise altitude / TAS the profile
    /// flies there.
    start_alt: f64,
    start_d: f64,
    cruise_alt: f64,
    cruise_tas: f64,
    /// Samples of `(distance-to-destination, altitude, true-airspeed, headwind)` from the field
    /// (d=0) up the descent, through cruise, and out the climb, in increasing `d`. Monotonic in `d`.
    /// The TAS is the phase-appropriate speed (descent/cruise/climb schedule) at that point, its
    /// Mach part in the air temperature there. The headwind is [`Self::wind`] at that point's
    /// distance and altitude, with a sample at every wind knot, so a query interpolates it like the
    /// TAS instead of reading the wind field again. All three are linearly interpolated between
    /// samples.
    samples: Vec<(f64, f64, f64, f64)>,
}

impl VerticalProfile {
    /// Build the profile. `start_alt_ft` / `start_d_nm` anchor the aircraft (for a ground or
    /// pre-file departure, pass surface altitude and the full route length). `cruise_req_ft` is
    /// the filed cruise altitude; `cruise_tas_kt` the (already filed-capped) cruise true airspeed.
    /// `wind` is the route's wind by distance-to-destination, on the same `d` axis.
    pub fn build(
        start_alt_ft: f64,
        start_d_nm: f64,
        arr_elev_ft: f64,
        cruise_req_ft: f64,
        cruise_tas_kt: f64,
        profile: &AircraftProfile,
        wind: RouteWind,
    ) -> Self {
        let arr_elev = arr_elev_ft.max(0.0);
        // A departure climbs from its own start altitude, even when the destination field is higher.
        let start_alt = start_alt_ft.max(0.0);
        let cruise_alt = cruise_req_ft
            .min(profile.service_ceiling_ft)
            .max(start_alt.max(arr_elev));
        let speeds = Speeds {
            profile,
            wind: &wind,
            cruise_tas: cruise_tas_kt.max(GS_FLOOR_KT),
        };

        // Descent leg (field → cruise): integrate altitude up from the field, accumulating the
        // horizontal distance each vertical step covers. Gives distance-to-destination at each alt.
        let desc = integrate_descent(arr_elev, cruise_alt, &speeds);
        let desc_dist = desc.last().map(|&(d, ..)| d).unwrap_or(0.0);

        // Climb leg (start_alt → cruise): distance the aircraft needs to reach cruise from here,
        // with the profile's per-band climb rates.
        let climb = integrate_climb(start_alt, cruise_alt, start_d_nm, &speeds);
        let climb_dist = climb.last().map(|&(d, ..)| d).unwrap_or(0.0);

        // Assemble samples in increasing distance-to-destination:
        //   0..desc_dist                     → descent curve (arr_elev up to cruise_alt)
        //   desc_dist..(start_d − climb_dist) → cruise
        //   top                              → climb curve anchored at (start_d, start_alt)
        let mut samples: Vec<(f64, f64, f64)> = Vec::new();
        let toc_d = start_d_nm - climb_dist; // distance-to-dest where cruise is reached

        if toc_d <= desc_dist {
            // Triangle: no room to cruise. Find the meeting altitude by walking descent + climb
            // outward until their combined distance fills the route; cap the peak there.
            let peak = triangle_peak(start_alt, arr_elev, cruise_alt, start_d_nm, &speeds);
            samples.extend(integrate_descent(arr_elev, peak, &speeds));
            let climb2 = integrate_climb(start_alt, peak, start_d_nm, &speeds);
            for &(cd, a, t) in climb2.iter().rev() {
                samples.push((start_d_nm - cd, a, t));
            }
        } else {
            samples.extend(desc.iter().copied());
            // The cruise leg: its TAS changes along the route only through the air temperature
            // under a cruise Mach, so a sample at each wind knot inside it (and just past a gap,
            // where the temperature steps) carries that change.
            for d in wind.knots.iter().flat_map(knot_sample_distances) {
                if d > desc_dist && d < toc_d {
                    samples.push((d, cruise_alt, speeds.cruise_at(d, cruise_alt)));
                }
            }
            samples.push((toc_d, cruise_alt, speeds.cruise_at(toc_d, cruise_alt))); // cruise start
            for &(cd, a, t) in climb.iter().rev() {
                samples.push((start_d_nm - cd, a, t));
            }
        }
        // Span the whole route, then sort & de-duplicate in `d`.
        let span_d = start_d_nm.max(desc_dist);
        samples.push((span_d, start_alt, speeds.cruise_at(span_d, start_alt)));
        samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        samples.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);

        let cruise_tas = speeds.cruise_at(start_d_nm, cruise_alt);
        let mut profile = Self {
            wind,
            gs_bias: 1.0,
            start_alt,
            start_d: start_d_nm,
            cruise_alt,
            cruise_tas,
            samples: samples
                .into_iter()
                .map(|(d, a, t)| (d, a, t, 0.0))
                .collect(),
        };
        profile.sample_headwinds();
        profile
    }

    /// Give every sample its headwind, adding a sample at each wind knot inside the profile so the
    /// headwind between samples is linear, as the wind field is between knots. A sample the
    /// cruise leg already holds keeps its TAS; one added here on the climb or descent interpolates
    /// it.
    fn sample_headwinds(&mut self) {
        if self.wind.is_still() {
            return;
        }
        let top = self.samples.last().map_or(0.0, |s| s.0);
        let extra: Vec<(f64, f64, f64, f64)> = self
            .wind
            .knots
            .iter()
            .flat_map(knot_sample_distances)
            .filter(|&d| d > 0.0 && d < top)
            .map(|d| (d, self.alt_at(d), self.interp(d, |s| s.2), 0.0))
            .collect();
        self.samples.extend(extra);
        self.samples
            .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        self.samples.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        for s in &mut self.samples {
            s.3 = self.wind.headwind(s.0, s.1).unwrap_or(0.0);
        }
    }

    /// Anchor the prediction to the aircraft's observed groundspeed (#313): when it is established
    /// at cruise (see [`ANCHOR_MIN_GS_KT`] / [`ANCHOR_ALT_TOLERANCE_FT`]), every predicted
    /// groundspeed is scaled by `observed / predicted-cruise-groundspeed`, clamped to
    /// ±[`ANCHOR_MAX_BIAS`]. Otherwise the profile is returned unchanged. One multiplier for the
    /// whole remaining flight, derived from the latest poll — nothing is stored between polls.
    pub fn anchor_to_observed_gs(mut self, observed_gs_kt: f64) -> Self {
        let established = observed_gs_kt >= ANCHOR_MIN_GS_KT
            && (self.start_alt - self.cruise_alt).abs() <= ANCHOR_ALT_TOLERANCE_FT;
        if established {
            // The cruise groundspeed where the aircraft is now, not the profile's speed at the exact
            // start point: an aircraft a few hundred feet below cruise sits on the climb curve's
            // (IAS-schedule) speed there.
            let hw = self.wind.headwind(self.start_d, self.cruise_alt);
            let predicted = effective_gs(self.cruise_tas, hw).max(GS_FLOOR_KT);
            self.gs_bias =
                (observed_gs_kt / predicted).clamp(1.0 - ANCHOR_MAX_BIAS, 1.0 + ANCHOR_MAX_BIAS);
        }
        self
    }

    /// The headwind (kt) the profile flies at `d`, at its altitude there; None in still air.
    fn headwind_at(&self, d_nm: f64) -> Option<f64> {
        (!self.wind.is_still()).then(|| self.interp(d_nm, |s| s.3))
    }

    /// The predicted groundspeed at `d`: interpolated TAS, corrected by the wind at `d` and the
    /// profile's altitude there, anchoring bias applied, floored at [`GS_FLOOR_KT`]. One search
    /// serves both the TAS and the headwind: this runs for every step of every timing query.
    fn gs_at(&self, d_nm: f64) -> f64 {
        let (i0, i1, t) = self.locate(d_nm);
        let field = |f: fn(&(f64, f64, f64, f64)) -> f64| {
            let (a, b) = (f(&self.samples[i0]), f(&self.samples[i1]));
            a + t * (b - a)
        };
        let hw = (!self.wind.is_still()).then(|| field(|s| s.3));
        (effective_gs(field(|s| s.2), hw) * self.gs_bias).max(GS_FLOOR_KT)
    }

    /// The samples bracketing distance-to-destination `d` and the fraction between them: equal
    /// indices (and `t` 0) at a sample or outside the samples' span, which holds the end value.
    fn locate(&self, d_nm: f64) -> (usize, usize, f64) {
        let d = d_nm.max(0.0);
        let last = self.samples.len().saturating_sub(1);
        match self
            .samples
            .binary_search_by(|s| s.0.partial_cmp(&d).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => (i, i, 0.0),
            Err(0) => (0, 0, 0.0),
            Err(i) if i > last => (last, last, 0.0),
            Err(i) => {
                let (s0, s1) = (&self.samples[i - 1], &self.samples[i]);
                let t = if (s1.0 - s0.0).abs() < 1e-9 {
                    0.0
                } else {
                    (d - s0.0) / (s1.0 - s0.0)
                };
                (i - 1, i, t)
            }
        }
    }

    /// Interpolate a sample field (`.1` altitude, `.2` TAS, `.3` headwind) at
    /// distance-to-destination `d`.
    fn interp(&self, d_nm: f64, field: impl Fn(&(f64, f64, f64, f64)) -> f64) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let (i0, i1, t) = self.locate(d_nm);
        let (a, b) = (field(&self.samples[i0]), field(&self.samples[i1]));
        a + t * (b - a)
    }

    /// Altitude (ft) at a distance-to-destination `d`.
    pub fn alt_at(&self, d_nm: f64) -> f64 {
        self.interp(d_nm, |s| s.1)
    }

    /// Predicted ground speed (kt) at a distance-to-destination `d` — the phase-appropriate TAS
    /// with wind (and any groundspeed anchoring) applied, floored at [`GS_FLOOR_KT`] exactly like
    /// [`Self::time_between`]'s internal integration, so a displayed speed never reads below what
    /// the paired ETA in the same row was actually timed against.
    ///
    /// This used to cap the result at the cruise groundspeed wherever the profile was still at
    /// cruise altitude, because [`Self::build`]'s samples made the cruise leg a TAS ramp (#355).
    /// That cap was a workaround over wrong samples, and deliberately did not apply to
    /// [`Self::time_between`], so ETAs stayed optimistic. #361 fixed the samples instead — the
    /// cruise leg is genuinely flat now and the profile never exceeds cruise TAS — so the cap is
    /// gone and this is simply the profile's own speed again.
    pub fn ground_speed_at(&self, d_nm: f64) -> f64 {
        self.gs_at(d_nm)
    }

    /// Seconds to fly from distance-to-destination `from_d` forward to `to_d` (`to_d < from_d`),
    /// integrating the phase-appropriate groundspeed in small steps.
    pub fn time_between(&self, from_d: f64, to_d: f64) -> f64 {
        let (hi, lo) = (from_d.max(to_d), from_d.min(to_d));
        let span = hi - lo;
        if span <= 0.0 {
            return 0.0;
        }
        let n = (span / HORIZ_STEP_NM).ceil().max(1.0) as usize;
        let step = span / n as f64;
        let mut secs = 0.0;
        for k in 0..n {
            let mid = hi - (k as f64 + 0.5) * step;
            secs += step / self.gs_at(mid) * 3600.0;
        }
        secs
    }

    /// Inverse of [`Self::time_between`] (#226's forward prediction scrubber): the
    /// distance-to-destination reached after `elapsed_sec` starting from `from_d`. `time_between`
    /// has no closed-form inverse (it's a numerical integral over climb/cruise/descent segments),
    /// but it's monotonic in the target distance, so this binary-searches it the same way
    /// [`triangle_peak`] searches a peak altitude. Clamped to `0.0` (already landed) when
    /// `elapsed_sec` reaches or exceeds the full remaining flight time.
    pub fn distance_after(&self, from_d: f64, elapsed_sec: f64) -> f64 {
        if elapsed_sec <= 0.0 {
            return from_d;
        }
        if elapsed_sec >= self.time_between(from_d, 0.0) {
            return 0.0;
        }
        let (mut lo, mut hi) = (0.0_f64, from_d);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if self.time_between(from_d, mid) > elapsed_sec {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }

    /// The distance-weighted mean headwind (kt) the profile flies between `from_d` and `to_d`, each
    /// step read at the profile's altitude there; a stretch the field does not cover counts as 0. A
    /// summary for the debug view; the timing itself reads the wind point by point. None in still
    /// air.
    pub fn mean_headwind(&self, from_d: f64, to_d: f64) -> Option<f64> {
        let (hi, lo) = (from_d.max(to_d), from_d.min(to_d));
        let n = ((hi - lo) / HORIZ_STEP_NM).ceil().max(1.0) as usize;
        let step = (hi - lo) / n as f64;
        let (mut sum, mut cnt) = (0.0, 0usize);
        for k in 0..n {
            let mid = hi - (k as f64 + 0.5) * step;
            if let Some(hw) = self.headwind_at(mid) {
                sum += hw;
                cnt += 1;
            }
        }
        (cnt > 0).then(|| sum / cnt as f64)
    }
}

/// The speed schedule a profile integrates with: the aircraft's phases, the wind along the route,
/// and the capped cruise TAS.
struct Speeds<'a> {
    profile: &'a AircraftProfile,
    wind: &'a RouteWind,
    cruise_tas: f64,
}

impl Speeds<'_> {
    /// The headwind and the air temperature (K) at `(d, alt)`: the field's temperature where it
    /// has one, ISA otherwise.
    fn air(&self, d: f64, alt: f64) -> (Option<f64>, f64) {
        let (hw, temp) = self.wind.air(d, alt);
        (hw, temp.unwrap_or_else(|| isa_temp_k(alt.max(0.0))))
    }

    /// Cruise TAS in air at `temp_k`, held down to the cruise Mach when one is set (Mach governs
    /// high up).
    fn cruise_in(&self, temp_k: f64) -> f64 {
        match self.profile.cruise_mach {
            Some(m) => self.cruise_tas.min(mach_to_tas_at(m, temp_k)),
            None => self.cruise_tas,
        }
    }

    /// The descent schedule at `alt` in air at `temp_k`, never faster than the aircraft cruises
    /// there (#361).
    ///
    /// Without the cap, a profile with no `desc_mach` resolves its IAS schedule to a TAS well above
    /// cruise up high (the default profile's 290 KIAS is 521 kt TAS at FL350 against a 440 kt
    /// cruise). The descent curve's top sample then sits at cruise altitude carrying that number,
    /// and `interp` ramps the cruise leg between two speeds at the same altitude, so `time_between`
    /// integrated an aircraft accelerating through cruise.
    fn descent_in(&self, alt: f64, temp_k: f64) -> f64 {
        let p = self.profile;
        phase_tas(p.desc_ias_lo, p.desc_ias_hi, p.desc_mach, alt, temp_k)
            .min(self.cruise_in(temp_k))
    }

    /// The climb schedule at `alt` in air at `temp_k`.
    fn climb_in(&self, alt: f64, temp_k: f64) -> f64 {
        let p = self.profile;
        phase_tas(p.climb_ias_lo, p.climb_ias_hi, p.climb_mach, alt, temp_k)
    }

    /// Cruise TAS at `(d, alt)`.
    fn cruise_at(&self, d: f64, alt: f64) -> f64 {
        self.cruise_in(self.air(d, alt).1)
    }

    /// The descent schedule at `(d, alt)`.
    fn descent_at(&self, d: f64, alt: f64) -> f64 {
        self.descent_in(alt, self.air(d, alt).1)
    }

    /// The climb schedule at `(d, alt)`.
    fn climb_at(&self, d: f64, alt: f64) -> f64 {
        self.climb_in(alt, self.air(d, alt).1)
    }
}

/// Groundspeed for a step flown at `tas` into `headwind`.
fn step_gs(tas: f64, headwind: Option<f64>) -> f64 {
    effective_gs(tas, headwind).max(GS_FLOOR_KT)
}

/// Integrate the descent from the field (`lo_alt`) up to `hi_alt` at the profile's descent rate,
/// returning `(distance_to_destination, altitude, true_airspeed)` samples. Each step's groundspeed
/// takes the wind and temperature at the step's own distance from the field and altitude.
fn integrate_descent(lo_alt: f64, hi_alt: f64, speeds: &Speeds) -> Vec<(f64, f64, f64)> {
    let fpm = speeds.profile.desc_fpm;
    let mut out = vec![(0.0, lo_alt, speeds.descent_at(0.0, lo_alt))];
    if hi_alt <= lo_alt || fpm <= 0.0 {
        return out;
    }
    let mut alt = lo_alt;
    let mut dist = 0.0;
    while alt < hi_alt {
        let top = (alt + VERT_STEP_FT).min(hi_alt);
        let mid = (alt + top) / 2.0;
        let step_sec = (top - alt) / fpm * 60.0;
        let (hw, temp) = speeds.air(dist, mid);
        dist += step_gs(speeds.descent_in(mid, temp), hw) * step_sec / 3600.0;
        alt = top;
        out.push((dist, alt, speeds.descent_at(dist, alt)));
    }
    out
}

/// Integrate a climb from `lo_alt` to `hi_alt` with the profile's per-band climb rates
/// (`climb_fpm_lo` below 10,000 ft, `climb_fpm_hi` above), starting `start_d` from the destination.
/// Returns `(distance_flown_from_start, altitude, tas)`; each step reads the wind and temperature
/// at `start_d − distance_flown` and its altitude.
fn integrate_climb(
    lo_alt: f64,
    hi_alt: f64,
    start_d: f64,
    speeds: &Speeds,
) -> Vec<(f64, f64, f64)> {
    let p = speeds.profile;
    let mut out = vec![(0.0, lo_alt, speeds.climb_at(start_d, lo_alt))];
    if hi_alt <= lo_alt {
        return out;
    }
    let mut alt = lo_alt;
    let mut dist = 0.0;
    while alt < hi_alt {
        let top = (alt + VERT_STEP_FT).min(hi_alt);
        let mid = (alt + top) / 2.0;
        let fpm = if mid < 10_000.0 {
            p.climb_fpm_lo
        } else {
            p.climb_fpm_hi
        }
        .max(1.0);
        let step_sec = (top - alt) / fpm * 60.0;
        let (hw, temp) = speeds.air(start_d - dist, mid);
        dist += step_gs(speeds.climb_in(mid, temp), hw) * step_sec / 3600.0;
        alt = top;
        out.push((dist, alt, speeds.climb_at(start_d - dist, alt)));
    }
    out
}

/// For a short hop with no room to cruise, find the altitude where climb-from-start and
/// descent-to-field meet within `route_nm` (binary search on the peak altitude).
fn triangle_peak(
    start_alt: f64,
    arr_elev: f64,
    cruise_cap: f64,
    route_nm: f64,
    speeds: &Speeds,
) -> f64 {
    let dist_for = |peak: f64| -> f64 {
        let c = integrate_climb(start_alt, peak, route_nm, speeds)
            .last()
            .map(|&(d, ..)| d)
            .unwrap_or(0.0);
        let d = integrate_descent(arr_elev, peak, speeds)
            .last()
            .map(|&(dd, ..)| dd)
            .unwrap_or(0.0);
        c + d
    };
    let (mut lo, mut hi) = (start_alt.max(arr_elev), cruise_cap);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if dist_for(mid) > route_nm {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    (lo + hi) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filed_altitude() {
        assert_eq!(parse_alt_ft("350"), 35000.0);
        assert_eq!(parse_alt_ft("FL350"), 35000.0);
        assert_eq!(parse_alt_ft("35000"), 35000.0);
        assert_eq!(parse_alt_ft(""), 35000.0);
        assert_eq!(parse_alt_ft("VFR"), 35000.0);
        assert_eq!(parse_alt_ft("80"), 8000.0);
    }

    #[test]
    fn tas_fallback_by_altitude() {
        assert_eq!(tas_or_default(450.0, 35000.0), 450.0);
        assert_eq!(tas_or_default(0.0, 35000.0), 440.0);
        assert_eq!(tas_or_default(0.0, 16000.0), 300.0);
        assert_eq!(tas_or_default(0.0, 5000.0), 170.0);
        // Garbage filed TAS (e.g. 4800) is rejected for the altitude default — the real-world bug
        // where a mis-filed cruise speed shot an aircraft to the front of the metering sequence.
        assert_eq!(tas_or_default(4800.0, 32000.0), 440.0);
        assert_eq!(tas_or_default(700.0, 32000.0), 700.0); // the cap itself is still believable
    }

    #[test]
    fn headwind_slows_tailwind_speeds() {
        assert_eq!(effective_gs(450.0, None), 450.0);
        assert_eq!(effective_gs(450.0, Some(50.0)), 400.0); // 50kt headwind
        assert_eq!(effective_gs(450.0, Some(-50.0)), 500.0); // 50kt tailwind
        // Absurd winds are clamped to ±60% of TAS.
        assert_eq!(effective_gs(450.0, Some(400.0)), 450.0 * 0.4);
    }

    // --- vertical-profile model ---

    /// A light-piston profile: slow, low ceiling, gentle climb — the C172 case.
    fn c172() -> AircraftProfile {
        AircraftProfile {
            climb_ias_lo: 75.0,
            climb_ias_hi: 90.0,
            climb_mach: None,
            climb_fpm_lo: 500.0,
            climb_fpm_hi: 500.0,
            cruise_tas: Some(110.0),
            cruise_mach: None,
            service_ceiling_ft: 14000.0,
            desc_mach: None,
            desc_ias_hi: 110.0,
            desc_ias_lo: 90.0,
            desc_fpm: 500.0,
        }
    }

    /// A heavy-jet profile with a Mach cruise — the B77W case.
    fn b77w() -> AircraftProfile {
        AircraftProfile {
            climb_ias_lo: 250.0,
            climb_ias_hi: 310.0,
            climb_mach: Some(0.84),
            climb_fpm_lo: 2500.0,
            climb_fpm_hi: 1800.0,
            cruise_tas: Some(490.0),
            cruise_mach: Some(0.84),
            service_ceiling_ft: 43000.0,
            desc_mach: Some(0.84),
            desc_ias_hi: 300.0,
            desc_ias_lo: 250.0,
            desc_fpm: 2000.0,
        }
    }

    #[test]
    fn ias_tas_and_mach_rise_with_altitude() {
        // TAS exceeds IAS at altitude (thinner air), and grows with height.
        assert!(ias_to_tas(250.0, 0.0) - 250.0 < 1.0);
        assert!(ias_to_tas(250.0, 35000.0) > 400.0);
        // Mach 0.80 ≈ 459 kt TAS at FL350 (colder → slower speed of sound than sea level).
        let t = mach_to_tas(0.80, 35000.0);
        assert!((450.0..475.0).contains(&t), "M0.80@FL350 ~ 460kt, got {t}");
    }

    #[test]
    fn light_aircraft_is_slower_than_a_heavy_jet() {
        // Same 200 nm departure leg (surface → cruise). The C172 must take far longer than the 777.
        let hw = RouteWind::still();
        let cessna = VerticalProfile::build(0.0, 200.0, 0.0, 12000.0, 110.0, &c172(), hw.clone());
        let heavy = VerticalProfile::build(0.0, 200.0, 0.0, 37000.0, 490.0, &b77w(), hw.clone());
        let t_c = cessna.time_between(200.0, 0.0);
        let t_h = heavy.time_between(200.0, 0.0);
        assert!(
            t_c > t_h * 2.0,
            "C172 {t_c}s should be far slower than B77W {t_h}s over the same leg"
        );
    }

    #[test]
    fn descent_is_modeled_below_top_of_descent() {
        // An airborne jet at FL350, 300 nm out. Altitude should be at cruise far out, and well
        // below cruise close in (past top-of-descent) — i.e. the descent phase exists.
        let p = b77w();
        let vp =
            VerticalProfile::build(35000.0, 300.0, 0.0, 35000.0, 480.0, &p, RouteWind::still());
        assert!(
            (vp.alt_at(300.0) - 35000.0).abs() < 1500.0,
            "cruising far out"
        );
        assert!(vp.alt_at(20.0) < 20000.0, "descending near the field");
        assert!(
            vp.alt_at(0.0) < 4000.0,
            "near field elevation at the threshold"
        );
    }

    #[test]
    fn service_ceiling_caps_cruise() {
        // Filed FL350 but a 14,000 ft ceiling → the C172 never models above its ceiling.
        let vp =
            VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 110.0, &c172(), RouteWind::still());
        let peak = (0..=300).map(|d| vp.alt_at(d as f64)).fold(0.0, f64::max);
        assert!(
            peak <= 14000.5,
            "cruise capped at the 14k ceiling, peak {peak}"
        );
    }

    // ---- distance_after: time_between's inverse, for #226's forward prediction scrubber ----

    #[test]
    fn distance_after_round_trips_through_time_between() {
        let vp = VerticalProfile::build(
            35000.0,
            300.0,
            0.0,
            35000.0,
            480.0,
            &b77w(),
            RouteWind::still(),
        );
        for to_d in [250.0, 150.0, 50.0, 10.0] {
            let elapsed = vp.time_between(300.0, to_d);
            let back = vp.distance_after(300.0, elapsed);
            assert!(
                (back - to_d).abs() < 0.5,
                "distance_after({elapsed}) = {back}, expected ~{to_d}"
            );
        }
    }

    #[test]
    fn distance_after_at_zero_elapsed_is_the_starting_distance() {
        let vp =
            VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), RouteWind::still());
        assert!((vp.distance_after(300.0, 0.0) - 300.0).abs() < 1e-6);
    }

    #[test]
    fn distance_after_clamps_to_zero_once_the_flight_would_have_landed() {
        let vp =
            VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), RouteWind::still());
        let total = vp.time_between(300.0, 0.0);
        assert_eq!(vp.distance_after(300.0, total), 0.0);
        assert_eq!(vp.distance_after(300.0, total + 3600.0), 0.0);
    }

    #[test]
    fn short_hop_never_reaches_cruise() {
        // 40 nm hop: a heavy can't climb to FL370 and back down in the distance, so the modeled
        // peak altitude stays well under the requested cruise (the triangle case).
        let vp =
            VerticalProfile::build(0.0, 40.0, 0.0, 37000.0, 490.0, &b77w(), RouteWind::still());
        let peak = (0..=40).map(|d| vp.alt_at(d as f64)).fold(0.0, f64::max);
        assert!(
            peak < 30000.0,
            "short hop peak {peak} should stay below cruise"
        );
        assert!(
            peak > 3000.0,
            "but it still climbs meaningfully, peak {peak}"
        );
    }

    #[test]
    fn headwind_slows_the_vertical_model_too() {
        let still = VerticalProfile::build(
            35000.0,
            400.0,
            0.0,
            35000.0,
            460.0,
            &b77w(),
            RouteWind::still(),
        );
        let hw = VerticalProfile::build(
            35000.0,
            400.0,
            0.0,
            35000.0,
            460.0,
            &b77w(),
            RouteWind::uniform(90.0),
        );
        assert!(hw.time_between(400.0, 0.0) > still.time_between(400.0, 0.0));
    }

    #[test]
    fn ground_speed_at_reflects_headwind_and_matches_alt_at_s_own_phase() {
        // Same cruise-altitude comparison as `headwind_slows_the_vertical_model_too`, but on the
        // per-point speed accessor the #225 per-fix debug table uses directly.
        let still = VerticalProfile::build(
            35000.0,
            400.0,
            0.0,
            35000.0,
            460.0,
            &b77w(),
            RouteWind::still(),
        );
        let hw = VerticalProfile::build(
            35000.0,
            400.0,
            0.0,
            35000.0,
            460.0,
            &b77w(),
            RouteWind::uniform(90.0),
        );
        assert!(hw.ground_speed_at(200.0) < still.ground_speed_at(200.0));

        // Near the field on a descent, ground speed should be well below the cruise TAS.
        let descending = VerticalProfile::build(
            35000.0,
            300.0,
            0.0,
            35000.0,
            480.0,
            &b77w(),
            RouteWind::still(),
        );
        assert!(
            descending.ground_speed_at(5.0) < 480.0,
            "expected a slower speed close to the field, got {}",
            descending.ground_speed_at(5.0)
        );
    }

    /// Regression (#225 rework): `ground_speed_at` must apply the same [`GS_FLOOR_KT`] floor
    /// `time_between`'s internal integration already does, or the debug table's KT column can read
    /// below the speed its own row's ETA was actually timed against — a strong headwind on the
    /// slow near-field approach TAS is exactly the case `effective_gs`'s own `tas * 0.4` clamp can
    /// push under the floor.
    #[test]
    fn ground_speed_at_never_reads_below_the_floor_time_between_uses() {
        // A slow GA aircraft's low descent IAS, plus a headwind, pushes `effective_gs`'s own
        // `tas * 0.4` clamp (≈36kt here) below GS_FLOOR_KT — exactly the case `time_between`'s
        // internal integration floors but `ground_speed_at` didn't.
        let slow_with_headwind = VerticalProfile::build(
            6000.0,
            30.0,
            0.0,
            6000.0,
            120.0,
            &c172(),
            RouteWind::uniform(60.0),
        );
        assert!(
            slow_with_headwind.ground_speed_at(1.0) >= GS_FLOOR_KT,
            "got {}",
            slow_with_headwind.ground_speed_at(1.0)
        );
    }

    #[test]
    fn default_profile_climb_still_costs_time_over_cruise() {
        // Sanity that the Default profile behaves like a climbing jet: a surface departure over
        // 300 nm takes longer than the same distance flown entirely at cruise.
        let p = AircraftProfile::default();
        let dep = VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 450.0, &p, RouteWind::still());
        let climb_time = dep.time_between(300.0, 0.0);
        let pure_cruise = 300.0 / 450.0 * 3600.0;
        assert!(
            climb_time > pure_cruise,
            "climb {climb_time} should exceed pure cruise {pure_cruise}"
        );
    }

    /// Regression (#355): anywhere the profile is still at cruise altitude, `ground_speed_at` must
    /// not read above the cruise groundspeed. `build` lays the descent curve down first and its top
    /// sample carries the *descent* schedule's TAS at cruise altitude (~519 kt for this profile at
    /// FL350 against a 440 kt cruise), while the only cruise sample sits at top-of-climb — so the
    /// raw interpolation ramps TAS across a leg whose altitude is flat. FCA MIT spacing is sized
    /// from this number, so an un-capped read under-provisions every enroute crossing: a 20 MIT fix
    /// 150 nm out would get `20 / 508 * 3600 = 142 s`, which the aircraft flies at its real 440 kt
    /// for 17.3 nm, not 20.
    #[test]
    fn ground_speed_at_never_reads_above_cruise_while_still_at_cruise_altitude() {
        let profile = AircraftProfile::default();
        let cruise_tas = 440.0;
        let vp = VerticalProfile::build(
            35_000.0,
            300.0,
            0.0,
            35_000.0,
            cruise_tas,
            &profile,
            RouteWind::still(),
        );
        let cruise_gs = effective_gs(cruise_tas, None);

        // Every point from the aircraft forward to top-of-descent is at cruise altitude.
        for d in [300.0, 250.0, 200.0, 150.0, 130.0] {
            assert!(
                (vp.alt_at(d) - 35_000.0).abs() < 1.0,
                "sanity: d={d} should still be at cruise altitude, got {:.0} ft",
                vp.alt_at(d)
            );
            assert!(
                vp.ground_speed_at(d) <= cruise_gs + 1e-6,
                "d={d} reads {:.1} kt, above the {cruise_gs:.1} kt cruise groundspeed",
                vp.ground_speed_at(d)
            );
        }

        // The cap must not flatten the descent: below cruise altitude the schedule still governs,
        // and the speed still falls away toward the field.
        assert!(vp.alt_at(20.0) < 35_000.0);
        assert!(
            vp.ground_speed_at(20.0) < cruise_gs * 0.8,
            "the descent must still slow down, got {:.1} kt",
            vp.ground_speed_at(20.0)
        );
    }

    // ---- groundspeed anchoring (#313) ----

    /// Still-air Default profile established at FL350 cruising 450 kt TAS, 400 nm out.
    fn at_cruise(start_alt: f64) -> VerticalProfile {
        VerticalProfile::build(
            start_alt,
            400.0,
            0.0,
            35000.0,
            450.0,
            &AircraftProfile::default(),
            RouteWind::still(),
        )
    }

    #[test]
    fn anchoring_scales_the_prediction_to_the_observed_groundspeed() {
        let raw = at_cruise(35000.0);
        let anchored = at_cruise(35000.0).anchor_to_observed_gs(495.0); // 10% faster than profile
        assert!((anchored.ground_speed_at(400.0) - 495.0).abs() < 1e-6);
        let (t_raw, t_anchored) = (
            raw.time_between(400.0, 0.0),
            anchored.time_between(400.0, 0.0),
        );
        assert!(
            (t_anchored - t_raw / 1.1).abs() < 1.0,
            "anchored {t_anchored}s should be ~{}s",
            t_raw / 1.1
        );
    }

    #[test]
    fn anchoring_bias_is_clamped_to_fifteen_percent() {
        let fast = at_cruise(35000.0).anchor_to_observed_gs(450.0 * 1.4);
        assert!((fast.ground_speed_at(400.0) - 450.0 * 1.15).abs() < 1e-6);
        let slow = at_cruise(35000.0).anchor_to_observed_gs(450.0 * 0.6);
        assert!((slow.ground_speed_at(400.0) - 450.0 * 0.85).abs() < 1e-6);
    }

    #[test]
    fn anchoring_trusts_only_an_aircraft_established_at_cruise() {
        let raw_time = |alt: f64| at_cruise(alt).time_between(400.0, 0.0);
        let anchored_time = |alt: f64, gs: f64| {
            at_cruise(alt)
                .anchor_to_observed_gs(gs)
                .time_between(400.0, 0.0)
        };

        // Climbing through FL240 at 400 kt: not established at cruise, flies the raw profile.
        assert_eq!(anchored_time(24000.0, 400.0), raw_time(24000.0));
        // At cruise but below the minimum trusted groundspeed.
        assert_eq!(anchored_time(35000.0, 240.0), raw_time(35000.0));
        // Just outside / inside the 300 ft cruise-altitude tolerance.
        assert_eq!(anchored_time(34650.0, 495.0), raw_time(34650.0));
        assert!(anchored_time(34750.0, 495.0) < raw_time(34750.0));
    }

    // ---- arrival-field elevation (#315) ----

    #[test]
    fn a_high_field_arrival_starts_its_descent_nearer_the_field() {
        // Same cruise and route into a sea-level field and into DEN (5,431 ft).
        let p = AircraftProfile::default();
        let sea =
            VerticalProfile::build(35000.0, 300.0, 0.0, 35000.0, 450.0, &p, RouteWind::still());
        let den = VerticalProfile::build(
            35000.0,
            300.0,
            5431.0,
            35000.0,
            450.0,
            &p,
            RouteWind::still(),
        );
        // Top of descent: the furthest point out still below cruise.
        let tod = |vp: &VerticalProfile| {
            (0..=3000)
                .map(|i| i as f64 / 10.0)
                .rfind(|&d| vp.alt_at(d) < 35000.0 - 1.0)
                .unwrap()
        };
        assert!(
            tod(&den) < tod(&sea) - 10.0,
            "DEN TOD {} nm should be well inside the sea-level TOD {} nm",
            tod(&den),
            tod(&sea)
        );
        // Both descents end at their own field.
        assert!((den.alt_at(0.0) - 5431.0).abs() < 1.0);
        assert!(sea.alt_at(0.0).abs() < 1.0);
    }

    #[test]
    fn a_departure_into_a_high_field_still_climbs_from_its_own_altitude() {
        // A sea-level departure into DEN must not start its climb at DEN's elevation.
        let p = AircraftProfile::default();
        let from_sea_level =
            VerticalProfile::build(0.0, 300.0, 5431.0, 35000.0, 450.0, &p, RouteWind::still());
        let from_field_height = VerticalProfile::build(
            5431.0,
            300.0,
            5431.0,
            35000.0,
            450.0,
            &p,
            RouteWind::still(),
        );
        assert!(from_sea_level.alt_at(300.0).abs() < 1.0);
        assert!(
            from_sea_level.time_between(300.0, 0.0) > from_field_height.time_between(300.0, 0.0),
            "climbing the extra 5,431 ft must cost time"
        );
    }

    // ---- #361: the cruise leg is flat ----------------------------------------------------------

    /// The regression this fix exists for. `build` assembles the descent curve first, and with no
    /// `desc_mach` the default profile's 290 KIAS schedule resolves to ~521 kt TAS at FL350 —
    /// above the 440 kt cruise. That number used to land on the descent curve's **top** sample, at
    /// cruise altitude, while the only cruise sample sat at top-of-climb with cruise TAS, so
    /// `interp` ramped between two speeds at the *same* altitude for the whole cruise leg
    /// (measured 440 → 462 → 485 → 508 → 519 kt at a flat 35,000 ft).
    ///
    /// Removing `.min(cruise_tas_at(alt))` from `desc_tas` restores that ramp and fails here.
    #[test]
    fn the_cruise_leg_is_flat_at_cruise_speed() {
        let profile = AircraftProfile::default();
        let cruise_tas = 440.0;
        let vp = VerticalProfile::build(
            35_000.0,
            300.0,
            0.0,
            35_000.0,
            cruise_tas,
            &profile,
            RouteWind::still(),
        );
        let cruise_gs = effective_gs(cruise_tas, None);

        // Every point still at cruise altitude must read exactly the cruise groundspeed.
        for d in [300.0_f64, 250.0, 200.0, 150.0, 130.0, 126.0] {
            assert!(
                (vp.alt_at(d) - 35_000.0).abs() < 1.0,
                "d={d} should still be at cruise altitude for this fixture, got {:.0} ft",
                vp.alt_at(d)
            );
            assert!(
                (vp.gs_at(d) - cruise_gs).abs() < 1.0,
                "d={d}: the cruise leg must be flat at {cruise_gs:.0} kt, got {:.1} kt",
                vp.gs_at(d)
            );
        }
    }

    /// The profile must never predict an aircraft flying faster than it cruises, at any point.
    /// This is the invariant the old `ground_speed_at` cap enforced as a workaround; now it holds
    /// in the samples themselves, so it is true for `time_between` (and therefore every ETA) too.
    #[test]
    fn the_profile_never_exceeds_cruise_speed() {
        let profile = AircraftProfile::default();
        let cruise_tas = 440.0;
        let route = 300.0;
        let vp = VerticalProfile::build(
            35_000.0,
            route,
            0.0,
            35_000.0,
            cruise_tas,
            &profile,
            RouteWind::still(),
        );
        let cruise_gs = effective_gs(cruise_tas, None);

        let mut d = route;
        while d >= 0.0 {
            assert!(
                vp.gs_at(d) <= cruise_gs + 1.0,
                "d={d:.0}: predicted {:.1} kt exceeds the {cruise_gs:.0} kt cruise groundspeed",
                vp.gs_at(d)
            );
            d -= 5.0;
        }

        // And the implied average over the whole route is therefore physically possible, which it
        // was not before: a 300 nm leg used to imply ~432 kt and a 1200 nm leg ~467 kt.
        let secs = vp.time_between(route, 0.0);
        let avg = route / (secs / 3600.0);
        assert!(
            avg <= cruise_gs,
            "implied average {avg:.1} kt over {route:.0} nm exceeds cruise {cruise_gs:.0} kt"
        );
    }

    /// The descent is still modelled — capping at cruise must not flatten the whole profile. Low
    /// down, where the schedule is genuinely slower than cruise, it takes over unchanged. This is
    /// also the value #355's MIT spacing depends on, so it is pinned here as well.
    #[test]
    fn the_descent_schedule_still_governs_below_cruise() {
        let profile = AircraftProfile::default();
        let vp = VerticalProfile::build(
            35_000.0,
            300.0,
            0.0,
            35_000.0,
            440.0,
            &profile,
            RouteWind::still(),
        );

        let near_field = vp.gs_at(20.0);
        assert!(
            (250.0..320.0).contains(&near_field),
            "a fix 20 nm out should read the arrival descent speed, got {near_field:.1} kt"
        );
        assert!(
            vp.gs_at(20.0) < vp.gs_at(80.0) && vp.gs_at(80.0) < vp.gs_at(150.0),
            "the descent must still slow monotonically toward the field: 20nm={:.1} 80nm={:.1} \
             150nm={:.1}",
            vp.gs_at(20.0),
            vp.gs_at(80.0),
            vp.gs_at(150.0)
        );
    }

    // ---- #836: wind and temperature read per segment, per altitude ----------------------------

    /// ISA temperature at FL350 (K): 288.15 − 1.9812 × 35.
    const ISA_FL350_K: f64 = 218.808;

    fn knot(d_nm: f64, levels: Vec<(f64, f64, Option<f64>)>) -> WindKnot {
        WindKnot { d_nm, levels }
    }

    /// The same wind and temperature at every altitude at each of these distances.
    fn flat(knots: &[(f64, f64, Option<f64>)]) -> RouteWind {
        RouteWind::new(
            knots
                .iter()
                .map(|&(d, hw, t)| knot(d, vec![(0.0, hw, t), (45_000.0, hw, t)]))
                .collect(),
        )
    }

    /// 80 kt on the nose at FL350, calm at and below 10,000 ft.
    fn shear() -> RouteWind {
        RouteWind::new(vec![knot(
            0.0,
            vec![
                (0.0, 0.0, None),
                (10_000.0, 0.0, None),
                (35_000.0, 80.0, None),
            ],
        )])
    }

    fn cruise(
        start_d: f64,
        tas: f64,
        profile: &AircraftProfile,
        wind: RouteWind,
    ) -> VerticalProfile {
        VerticalProfile::build(35_000.0, start_d, 0.0, 35_000.0, tas, profile, wind)
    }

    /// The furthest distance-to-destination still below cruise: top of descent.
    fn top_of_descent(vp: &VerticalProfile, max_d: f64) -> f64 {
        let mut d = 0.0;
        while d < max_d && vp.alt_at(d + 0.1) < 34_999.0 {
            d += 0.1;
        }
        d
    }

    // AC3: an aircraft's wind at FL350 does not apply on its approach at 3,000 ft.
    #[test]
    fn a_wind_strong_at_cruise_and_calm_low_slows_cruise_but_not_the_approach() {
        let p = AircraftProfile::default();
        let windy = cruise(300.0, 440.0, &p, shear());
        let still = cruise(300.0, 440.0, &p, RouteWind::still());
        let one_value = cruise(300.0, 440.0, &p, RouteWind::uniform(80.0));
        assert!((windy.ground_speed_at(250.0) - 360.0).abs() < 0.5);
        assert!((still.ground_speed_at(250.0) - 440.0).abs() < 0.5);
        assert!((windy.ground_speed_at(5.0) - still.ground_speed_at(5.0)).abs() < 0.5);
        let t = windy.time_between(300.0, 0.0);
        assert!(t > still.time_between(300.0, 0.0));
        assert!(
            t < one_value.time_between(300.0, 0.0),
            "the descent through calmer air is quicker than one cruise-level headwind all the way"
        );
    }

    // AC3: a headwind on the first half of the route only slows the first half.
    #[test]
    fn wind_that_differs_along_the_route_is_applied_where_it_blows() {
        let p = AircraftProfile::default();
        let wind = flat(&[
            (0.0, 0.0, None),
            (199.0, 0.0, None),
            (201.0, 100.0, None),
            (400.0, 100.0, None),
        ]);
        let windy = cruise(400.0, 440.0, &p, wind);
        let still = cruise(400.0, 440.0, &p, RouteWind::still());
        // 199 nm at 340 kt against 440 kt: 2,107.1 − 1,628.2 s.
        let extra = windy.time_between(400.0, 201.0) - still.time_between(400.0, 201.0);
        assert!((extra - 478.9).abs() < 3.0, "{extra}");
        let near = windy.time_between(199.0, 0.0) - still.time_between(199.0, 0.0);
        assert!(near.abs() < 1.0, "{near}");
    }

    // AC3: the per-segment answer is not the route-mean answer.
    #[test]
    fn a_varying_wind_is_not_the_same_as_its_route_mean() {
        let p = AircraftProfile::default();
        let wind = flat(&[
            (0.0, 0.0, None),
            (199.0, 0.0, None),
            (201.0, 100.0, None),
            (400.0, 100.0, None),
        ]);
        let per_segment = cruise(400.0, 440.0, &p, wind).time_between(400.0, 0.0);
        let mean = cruise(400.0, 440.0, &p, RouteWind::uniform(50.0)).time_between(400.0, 0.0);
        // 1/(TAS − hw) is convex: at a flat 440 kt, 200/340 + 200/440 h is 3,754 s against
        // 400/390 h = 3,692 s.
        assert!(per_segment > mean + 30.0, "{per_segment} vs {mean}");
    }

    // AC3: descent integration reads the wind at each step's own altitude.
    #[test]
    fn a_low_level_tailwind_moves_top_of_descent_further_out() {
        let p = AircraftProfile::default();
        let low_tail = RouteWind::new(vec![knot(
            0.0,
            vec![
                (0.0, -60.0, None),
                (9_000.0, -60.0, None),
                (11_000.0, 0.0, None),
                (35_000.0, 0.0, None),
            ],
        )]);
        let windy = cruise(300.0, 440.0, &p, low_tail);
        let still = cruise(300.0, 440.0, &p, RouteWind::still());
        let moved = top_of_descent(&windy, 300.0) - top_of_descent(&still, 300.0);
        // Below 10,000 ft the descent takes ~5.5 min at 1,800 fpm; 60 kt of tailwind adds ~5.5 nm.
        assert!(moved > 4.0, "top of descent moved {moved} nm");
    }

    // AC3: climb integration reads the wind at its own d. 60 kt on the nose for the first 20 nm
    // only (calm from 30 nm out) costs the climb those miles' share; read at the departure for every
    // step, it would cost the whole climb's, over 15 nm.
    #[test]
    fn a_departure_end_headwind_lengthens_the_climb_not_the_descent() {
        let p = AircraftProfile::default();
        let wind = flat(&[
            (0.0, 0.0, None),
            (270.0, 0.0, None),
            (280.0, 60.0, None),
            (300.0, 60.0, None),
        ]);
        let build = |w| VerticalProfile::build(0.0, 300.0, 0.0, 35_000.0, 440.0, &p, w);
        let (windy, still) = (build(wind), build(RouteWind::still()));
        let climb_nm = |vp: &VerticalProfile| {
            let mut d = 300.0;
            while d > 0.0 && vp.alt_at(d) < 34_999.0 {
                d -= 0.1;
            }
            300.0 - d
        };
        let shorter = climb_nm(&still) - climb_nm(&windy);
        assert!(
            (3.0..10.0).contains(&shorter),
            "the climb covers {shorter} nm less ground"
        );
        for d in [20.0, 50.0, 80.0] {
            assert!(
                (windy.alt_at(d) - still.alt_at(d)).abs() < 10.0,
                "descent at {d} nm"
            );
        }
    }

    // The field's temperature drives the Mach TAS.
    #[test]
    fn a_cruise_mach_flies_faster_in_warm_air_than_in_isa() {
        let warm = RouteWind::new(vec![knot(
            0.0,
            vec![(35_000.0, 0.0, Some(ISA_FL350_K + 15.0))],
        )]);
        let hot = cruise(400.0, 520.0, &b77w(), warm);
        let isa = cruise(400.0, 520.0, &b77w(), RouteWind::still());
        // 661.4788 kt × 0.84 × √(T / 288.15).
        assert!(
            (hot.ground_speed_at(350.0) - 500.5).abs() < 0.3,
            "{}",
            hot.ground_speed_at(350.0)
        );
        assert!(
            (isa.ground_speed_at(350.0) - 484.2).abs() < 0.3,
            "{}",
            isa.ground_speed_at(350.0)
        );
    }

    // mach_to_tas_at against literals.
    #[test]
    fn mach_to_tas_at_matches_the_speed_of_sound_at_that_temperature() {
        assert!((mach_to_tas_at(1.0, 288.15) - 661.4788).abs() < 1e-9);
        assert!((mach_to_tas_at(0.84, ISA_FL350_K) - 484.2).abs() < 0.1);
        assert!((mach_to_tas(0.84, 35_000.0) - mach_to_tas_at(0.84, ISA_FL350_K)).abs() < 1e-9);
    }

    // ISA where the field has no temperature (FB 3,000 ft; a RAP point with TMP missing).
    #[test]
    fn a_field_without_temperatures_flies_mach_on_isa() {
        let wind = RouteWind::new(vec![knot(0.0, vec![(35_000.0, 40.0, None)])]);
        let vp = cruise(400.0, 520.0, &b77w(), wind);
        assert!((vp.ground_speed_at(350.0) - (484.2 - 40.0)).abs() < 0.3);
    }

    // Temperature reaches the Mach portion only: an IAS-only profile ignores it.
    #[test]
    fn warm_air_does_not_change_an_ias_only_profile() {
        let warm = flat(&[(0.0, 0.0, Some(ISA_FL350_K + 20.0))]);
        for (p, tas, alt) in [
            (c172(), 110.0, 12_000.0),
            (AircraftProfile::default(), 440.0, 35_000.0),
        ] {
            let build = |w| VerticalProfile::build(0.0, 300.0, 0.0, alt, tas, &p, w);
            let (hot, isa) = (build(warm.clone()), build(RouteWind::still()));
            assert!((hot.time_between(300.0, 0.0) - isa.time_between(300.0, 0.0)).abs() < 1e-9);
        }
    }

    // The cruise leg carries a sample at each knot, so a temperature change mid-cruise changes
    // the Mach TAS there (not a ramp between top of climb and top of descent).
    #[test]
    fn the_cruise_leg_changes_speed_where_the_temperature_changes() {
        let wind = RouteWind::new(
            [
                (0.0, ISA_FL350_K),
                (300.0, ISA_FL350_K),
                (301.0, ISA_FL350_K + 15.0),
                (600.0, ISA_FL350_K + 15.0),
            ]
            .iter()
            .map(|&(d, t)| knot(d, vec![(35_000.0, 0.0, Some(t))]))
            .collect(),
        );
        let vp = cruise(600.0, 520.0, &b77w(), wind);
        assert!(
            (vp.ground_speed_at(450.0) - 500.5).abs() < 0.3,
            "{}",
            vp.ground_speed_at(450.0)
        );
        assert!(
            (vp.ground_speed_at(250.0) - 484.2).abs() < 0.3,
            "{}",
            vp.ground_speed_at(250.0)
        );
    }

    // Anchoring (#313) compares the observed groundspeed with the prediction where the aircraft
    // is, not with a route mean.
    #[test]
    fn anchoring_reads_the_headwind_where_the_aircraft_is() {
        let p = AircraftProfile::default();
        let wind = flat(&[
            (0.0, 100.0, None),
            (200.0, 100.0, None),
            (300.0, 0.0, None),
            (400.0, 0.0, None),
        ]);
        let vp = cruise(400.0, 450.0, &p, wind.clone()).anchor_to_observed_gs(450.0);
        assert!(
            (vp.ground_speed_at(400.0) - 450.0).abs() < 1e-6,
            "{}",
            vp.ground_speed_at(400.0)
        );
        let fast = cruise(400.0, 450.0, &p, wind).anchor_to_observed_gs(495.0);
        assert!((fast.ground_speed_at(400.0) - 495.0).abs() < 1e-6);
    }

    // RouteWind sampling between and outside knots.
    #[test]
    fn route_wind_interpolates_between_knots_and_holds_outside_them() {
        let w = RouteWind::new(vec![
            knot(100.0, vec![(0.0, 20.0, None)]),
            knot(200.0, vec![(0.0, 60.0, None)]),
        ]);
        assert_eq!(w.headwind(150.0, 0.0), Some(40.0));
        assert_eq!(w.headwind(100.0, 0.0), Some(20.0));
        assert_eq!(w.headwind(50.0, 0.0), Some(20.0));
        assert_eq!(w.headwind(250.0, 0.0), Some(60.0));
    }

    // RouteWind: one knot lacks temperature; the other's is used rather than none.
    #[test]
    fn route_wind_falls_back_to_the_knot_that_has_the_field() {
        let w = RouteWind::new(vec![
            knot(100.0, vec![(35_000.0, 20.0, Some(220.0))]),
            knot(200.0, vec![(35_000.0, 60.0, None)]),
        ]);
        assert_eq!(w.temp_k(150.0, 35_000.0), Some(220.0));
        assert_eq!(w.headwind(150.0, 35_000.0), Some(40.0));
    }

    // by_altitude: between levels and outside them.
    #[test]
    fn route_wind_interpolates_between_levels_and_holds_outside_them() {
        let w = RouteWind::new(vec![knot(
            0.0,
            vec![(30_000.0, 50.0, Some(230.0)), (10_000.0, 10.0, Some(250.0))],
        )]);
        assert_eq!(w.headwind(0.0, 20_000.0), Some(30.0));
        assert_eq!(w.headwind(0.0, 5_000.0), Some(10.0));
        assert_eq!(w.headwind(0.0, 40_000.0), Some(50.0));
        assert_eq!(w.temp_k(0.0, 20_000.0), Some(240.0));
    }

    // RouteWind::new normalizes its input.
    #[test]
    fn route_wind_new_sorts_knots_and_reads_an_empty_knot_as_a_gap() {
        let w = RouteWind::new(vec![
            knot(300.0, vec![(0.0, 90.0, None)]),
            knot(0.0, vec![(0.0, 30.0, None)]),
            knot(100.0, vec![]),
            knot(200.0, vec![]),
        ]);
        // Sorted: 0 (30 kt), 100 (gap), 200 (gap), 300 (90 kt). Next to a covered knot its value
        // holds up to the gap; between two gaps the air is still.
        assert_eq!(w.headwind(50.0, 0.0), Some(30.0));
        assert_eq!(w.headwind(150.0, 0.0), None);
        assert_eq!(w.headwind(250.0, 0.0), Some(90.0));
        assert!(!w.is_still());
        assert!(RouteWind::new(vec![]).is_still());
        assert!(RouteWind::new(vec![knot(10.0, vec![]), knot(20.0, vec![])]).is_still());
    }

    /// A field covering only 450 and 500 nm out with these levels, every other knot (each 50 nm)
    /// a gap: covered values hold from 400 to 550 nm, and the air is still beyond.
    fn covered_450_to_500(levels: Vec<(f64, f64, Option<f64>)>) -> RouteWind {
        let mut knots = vec![knot(500.0, levels.clone()), knot(450.0, levels)];
        for d in [
            0.0, 50.0, 100.0, 150.0, 200.0, 250.0, 300.0, 350.0, 400.0, 550.0, 600.0,
        ] {
            knots.push(knot(d, vec![]));
        }
        RouteWind::new(knots)
    }

    // The profile flies the gap rule: a covered wind holds up to the first gap knot past the field
    // edge and stops there, on the way out of the field and on the way in.
    #[test]
    fn the_profile_stops_a_covered_wind_at_the_first_gap_past_the_field() {
        let p = AircraftProfile::default();
        let vp = cruise(
            600.0,
            440.0,
            &p,
            covered_450_to_500(vec![(0.0, 80.0, None)]),
        );
        let still = cruise(600.0, 440.0, &p, RouteWind::still());
        for d in [575.0, 375.0] {
            assert!(
                (vp.ground_speed_at(d) - still.ground_speed_at(d)).abs() < 1e-6,
                "{d} nm is past the first gap: still air, got {}",
                vp.ground_speed_at(d)
            );
        }
        for d in [525.0, 475.0, 425.0] {
            assert!(
                (vp.ground_speed_at(d) - (still.ground_speed_at(d) - 80.0)).abs() < 1e-6,
                "{d} nm holds the covered 80 kt, got {}",
                vp.ground_speed_at(d)
            );
        }
        let gap_span = vp.time_between(600.0, 550.0);
        assert!(
            (gap_span - still.time_between(600.0, 550.0)).abs() < 0.5,
            "the span between two gaps is timed in still air: {gap_span}"
        );
    }

    // Under a cruise Mach the temperature steps at a gap as the wind does: past the first gap the
    // TAS is ISA's, and up to it the covered warm air's.
    #[test]
    fn the_profile_stops_a_covered_temperature_at_the_first_gap_past_the_field() {
        let warm = vec![(35_000.0, 0.0, Some(ISA_FL350_K + 15.0))];
        let vp = cruise(600.0, 520.0, &b77w(), covered_450_to_500(warm.clone()));
        let isa = cruise(600.0, 520.0, &b77w(), RouteWind::still());
        let hot = cruise(600.0, 520.0, &b77w(), RouteWind::new(vec![knot(0.0, warm)]));
        assert!(hot.ground_speed_at(525.0) > isa.ground_speed_at(525.0) + 10.0);
        for d in [575.0, 375.0] {
            assert!(
                (vp.ground_speed_at(d) - isa.ground_speed_at(d)).abs() < 1e-6,
                "{d} nm is past the first gap: ISA, got {}",
                vp.ground_speed_at(d)
            );
        }
        for d in [525.0, 475.0, 425.0] {
            assert!(
                (vp.ground_speed_at(d) - hot.ground_speed_at(d)).abs() < 1e-6,
                "{d} nm holds the covered warm air, got {}",
                vp.ground_speed_at(d)
            );
        }
    }

    // The debug mean counts a stretch the field does not cover as still air: 80 kt over the 150 nm
    // the field covers (400 to 550 nm) and 0 over the other 100 nm of 600 to 350 nm.
    #[test]
    fn mean_headwind_counts_a_gap_as_still_air() {
        let p = AircraftProfile::default();
        let vp = cruise(
            600.0,
            440.0,
            &p,
            covered_450_to_500(vec![(0.0, 80.0, None)]),
        );
        let mean = vp.mean_headwind(600.0, 350.0).unwrap();
        assert!((mean - 48.0).abs() < 0.5, "{mean}");
        assert!((vp.mean_headwind(550.0, 400.0).unwrap() - 80.0).abs() < 0.5);
    }

    // Still air: no wind, no temperature.
    #[test]
    fn still_air_reads_no_wind_and_no_temperature() {
        let s = RouteWind::still();
        assert!(s.is_still());
        assert_eq!(s.headwind(100.0, 35_000.0), None);
        assert_eq!(s.temp_k(100.0, 35_000.0), None);
        assert!(!RouteWind::uniform(0.0).is_still());
    }

    // The debug view's headwind is the profile's own distance-weighted mean.
    #[test]
    fn mean_headwind_weights_each_step_at_the_profiles_altitude_there() {
        let p = AircraftProfile::default();
        let vp = cruise(300.0, 440.0, &p, shear());
        let mean = vp.mean_headwind(300.0, 0.0).unwrap();
        assert!(
            mean > 50.0 && mean < 78.0,
            "the descent reads less than 80: {mean}"
        );
        assert!((vp.mean_headwind(300.0, 250.0).unwrap() - 80.0).abs() < 0.5);
        assert_eq!(
            cruise(300.0, 440.0, &p, RouteWind::still()).mean_headwind(300.0, 0.0),
            None
        );
    }

    /// ISA at FL350, and 15 °C warmer than ISA from 28,000 to 34,000 ft: the Mach part of the
    /// b77w's climb and descent (its 310/300 KIAS schedules cross M0.84 near FL300) sits in warm
    /// air while its cruise at FL350 does not.
    fn warm_below_cruise() -> RouteWind {
        let isa = |ft: f64| 288.15 - 0.0019812 * ft;
        RouteWind::new(vec![knot(
            0.0,
            vec![
                (28_000.0, 0.0, Some(isa(28_000.0) + 15.0)),
                (34_000.0, 0.0, Some(isa(34_000.0) + 15.0)),
                (35_000.0, 0.0, Some(ISA_FL350_K)),
            ],
        )])
    }

    // The field's temperature drives the Mach part of the descent: warm air is faster there.
    #[test]
    fn a_descent_mach_flies_faster_in_warm_air() {
        let build = |w| VerticalProfile::build(0.0, 300.0, 0.0, 35_000.0, 520.0, &b77w(), w);
        let (warm, isa) = (build(warm_below_cruise()), build(RouteWind::still()));
        // The last 150 nm hold the whole descent, which is integrated from the field and does not
        // depend on the climb.
        let saved = isa.time_between(150.0, 0.0) - warm.time_between(150.0, 0.0);
        assert!(saved > 0.5, "the warm Mach descent saves {saved} s");
    }

    // The descent reads the temperature at each step's own distance: warm air only from 60 nm
    // out (where the Mach part of the descent is) still speeds it up.
    #[test]
    fn a_descent_reads_the_temperature_where_each_step_is() {
        let isa = |ft: f64| 288.15 - 0.0019812 * ft;
        let band = |dt: f64| {
            vec![
                (28_000.0, 0.0, Some(isa(28_000.0) + dt)),
                (34_000.0, 0.0, Some(isa(34_000.0) + dt)),
                (35_000.0, 0.0, Some(ISA_FL350_K)),
            ]
        };
        let far_warm = RouteWind::new(vec![
            knot(0.0, band(0.0)),
            knot(50.0, band(0.0)),
            knot(60.0, band(15.0)),
            knot(300.0, band(15.0)),
        ]);
        let build = |w| VerticalProfile::build(0.0, 300.0, 0.0, 35_000.0, 520.0, &b77w(), w);
        let (warm, isa_p) = (build(far_warm), build(RouteWind::still()));
        let saved = isa_p.time_between(100.0, 0.0) - warm.time_between(100.0, 0.0);
        assert!(saved > 0.5, "warm upper descent saves {saved} s");
        // The descent's shape too: flown faster through the warm Mach band, it covers more ground
        // per foot, so top of descent sits further out.
        let tod = |vp: &VerticalProfile| {
            let (mut lo, mut hi) = (0.0, 299.0);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if vp.alt_at(mid) < 34_999.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let further = tod(&warm) - tod(&isa_p);
        assert!(further > 0.05, "top of descent moved out {further} nm");
    }

    // The same on the climb: the Mach part of it is faster in warm air.
    #[test]
    fn a_climb_mach_flies_faster_in_warm_air() {
        let build = |w| VerticalProfile::build(0.0, 300.0, 0.0, 35_000.0, 520.0, &b77w(), w);
        let (warm, isa) = (build(warm_below_cruise()), build(RouteWind::still()));
        // The first 150 nm hold the climb and the start of the ISA cruise.
        let saved = isa.time_between(300.0, 150.0) - warm.time_between(300.0, 150.0);
        assert!(saved > 1.0, "the warm Mach climb saves {saved} s");
    }

    // The climb reads the temperature where each step is: warm air at the departure end only, calm
    // ISA from 30 nm out, leaves the Mach part of the climb (well past 30 nm) on ISA.
    #[test]
    fn a_climb_reads_the_temperature_where_each_step_is() {
        let isa = |ft: f64| 288.15 - 0.0019812 * ft;
        let levels = |dt: f64| {
            vec![
                (28_000.0, 0.0, Some(isa(28_000.0) + dt)),
                (34_000.0, 0.0, Some(isa(34_000.0) + dt)),
                (35_000.0, 0.0, Some(ISA_FL350_K + dt)),
            ]
        };
        let wind = RouteWind::new(vec![
            knot(0.0, levels(0.0)),
            knot(270.0, levels(0.0)),
            knot(300.0, levels(15.0)),
        ]);
        let build = |w| VerticalProfile::build(0.0, 300.0, 0.0, 35_000.0, 520.0, &b77w(), w);
        let (warm_start, isa_air) = (build(wind), build(RouteWind::still()));
        let gained = isa_air.time_between(300.0, 150.0) - warm_start.time_between(300.0, 150.0);
        assert!(gained.abs() < 0.05, "the Mach climb is on ISA: {gained} s");
    }

    // AC3 inside the descent: a headwind from 60 nm out and calm inside 50 nm moves top of
    // descent in, because the descent is integrated through the wind at each step's own distance.
    #[test]
    fn a_headwind_on_the_upper_descent_moves_top_of_descent_in() {
        let p = AircraftProfile::default();
        let wind = flat(&[
            (0.0, 0.0, None),
            (50.0, 0.0, None),
            (60.0, 80.0, None),
            (300.0, 80.0, None),
        ]);
        let (windy, still) = (
            cruise(300.0, 440.0, &p, wind),
            cruise(300.0, 440.0, &p, RouteWind::still()),
        );
        let moved = top_of_descent(&still, 300.0) - top_of_descent(&windy, 300.0);
        assert!(moved > 2.0, "top of descent moved in {moved} nm");
        // Inside 50 nm both fly the same calm descent.
        assert!((windy.alt_at(30.0) - still.alt_at(30.0)).abs() < 1.0);
    }

    // A wind knot inside the descent adds no cruise-altitude sample there.
    #[test]
    fn a_wind_knot_inside_the_descent_keeps_the_descent_shape() {
        let p = AircraftProfile::default();
        let wind = flat(&[(0.0, 10.0, None), (30.0, 10.0, None), (300.0, 10.0, None)]);
        let vp = cruise(300.0, 440.0, &p, wind);
        assert!(
            vp.alt_at(30.0) < 20_000.0,
            "30 nm out reads {} ft",
            vp.alt_at(30.0)
        );
    }

    // A temperature at or below absolute zero is clamped to 1 K rather than taking a square root
    // of a negative number.
    #[test]
    fn mach_to_tas_at_clamps_a_non_physical_temperature() {
        // 0.8 × 661.4788 × √(1 / 288.15)
        let tas = mach_to_tas_at(0.8, -10.0);
        assert!((tas - 31.17).abs() < 0.01, "{tas}");
    }

    // RouteWind: the first knot lacks the field; the second's is used.
    #[test]
    fn route_wind_falls_back_to_the_second_knot_when_the_first_lacks_the_field() {
        let w = RouteWind::new(vec![
            knot(100.0, vec![(35_000.0, 20.0, None)]),
            knot(200.0, vec![(35_000.0, 60.0, Some(220.0))]),
        ]);
        assert_eq!(w.temp_k(150.0, 35_000.0), Some(220.0));
    }
}
