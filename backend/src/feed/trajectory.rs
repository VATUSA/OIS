//! Shared trajectory / ETA model — a per-aircraft vertical-profile integrator with winds-aloft
//! correction. This is the single predictor used by every surface that estimates where connected
//! aircraft will be at a given time (FCA metering, airport-flow demand, runway ETE), so those
//! surfaces stay consistent. The crossing/arrival geometry ports from vatflow's `fca-metering.js`;
//! the performance model is OIS's own configurable [`AircraftProfile`] / [`VerticalProfile`].
//!
//! A profile integrates a climb (per-band IAS schedule + rates), a cruise (TAS or Mach), and a
//! descent to a real top-of-descent — with an ISA Mach↔TAS crossover — corrected for the mean
//! route headwind ([`crate::feed::winds`]). It fails safe to still air when winds are unavailable.

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

/// True airspeed (kt) for a Mach number at altitude: TAS = M · a, a = a₀·√(T/T₀).
pub fn mach_to_tas(mach: f64, alt_ft: f64) -> f64 {
    mach * SPEED_OF_SOUND_SL_KT * (isa_temp_k(alt_ft.max(0.0)) / ISA_T0_K).sqrt()
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
/// IAS wins low). `ias` is the schedule speed for the altitude band; `mach` the phase Mach.
fn phase_tas(ias_lo: f64, ias_hi: f64, mach: Option<f64>, alt_ft: f64) -> f64 {
    let ias = if alt_ft < 10_000.0 { ias_lo } else { ias_hi };
    let tas_ias = ias_to_tas(ias, alt_ft);
    match mach {
        Some(m) => tas_ias.min(mach_to_tas(m, alt_ft)),
        None => tas_ias,
    }
}

/// Groundspeed floor so a stalled step can't divide by ~zero.
const GS_FLOOR_KT: f64 = 60.0;
/// Vertical integration step (ft) for building the climb/descent altitude-vs-distance curves.
const VERT_STEP_FT: f64 = 500.0;
/// Horizontal integration step (nm) for timing a distance span across the profile.
const HORIZ_STEP_NM: f64 = 3.0;

/// A resolved vertical flight profile, anchored at the aircraft's current state and running
/// forward to the destination. Built once per aircraft, then queried for the altitude at, and
/// the time to reach, any point measured as distance-to-destination.
///
/// Distances are **nm-to-destination** (`d`): `d = 0` is the field, increasing back along the
/// route. The aircraft is at `(start_alt, start_d)`. Ahead of it: climb from `start_alt` to the
/// achieved cruise altitude, cruise, then descend from top-of-descent to the field.
pub struct VerticalProfile {
    headwind: Option<f64>,
    /// Samples of `(distance-to-destination, altitude, true-airspeed)` from the field (d=0) up the
    /// descent, through cruise, and out the climb, in increasing `d`. Monotonic in `d`. The TAS is
    /// the phase-appropriate speed (descent/cruise/climb schedule) at that point; wind is applied
    /// at query time. Both altitude and TAS are linearly interpolated between samples.
    samples: Vec<(f64, f64, f64)>,
}

impl VerticalProfile {
    /// Build the profile. `start_alt_ft` / `start_d_nm` anchor the aircraft (for a ground or
    /// pre-file departure, pass surface altitude and the full route length). `cruise_req_ft` is
    /// the filed cruise altitude; `cruise_tas_kt` the (already filed-capped) cruise true airspeed.
    pub fn build(
        start_alt_ft: f64,
        start_d_nm: f64,
        arr_elev_ft: f64,
        cruise_req_ft: f64,
        cruise_tas_kt: f64,
        profile: &AircraftProfile,
        headwind: Option<f64>,
    ) -> Self {
        let arr_elev = arr_elev_ft.max(0.0);
        let start_alt = start_alt_ft.max(arr_elev);
        let cruise_alt = cruise_req_ft
            .min(profile.service_ceiling_ft)
            .max(start_alt.max(arr_elev));
        let cruise_tas = cruise_tas_kt.max(GS_FLOOR_KT);
        // Cruise TAS held down to the cruise Mach when one is set (Mach governs high up).
        let cruise_tas_at = |alt: f64| match profile.cruise_mach {
            Some(m) => cruise_tas.min(mach_to_tas(m, alt)),
            None => cruise_tas,
        };
        let desc_tas = |alt: f64| {
            phase_tas(
                profile.desc_ias_lo,
                profile.desc_ias_hi,
                profile.desc_mach,
                alt,
            )
        };

        // Descent leg (field → cruise): integrate altitude up from the field, accumulating the
        // horizontal distance each vertical step covers. Gives distance-to-destination at each alt.
        let desc = integrate_vertical(arr_elev, cruise_alt, profile.desc_fpm, headwind, desc_tas);
        let desc_dist = desc.last().map(|&(d, ..)| d).unwrap_or(0.0);

        // Climb leg (start_alt → cruise): distance the aircraft needs to reach cruise from here,
        // with the profile's per-band climb rates.
        let climb = integrate_climb(start_alt, cruise_alt, profile, headwind);
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
            let peak = triangle_peak(
                start_alt, arr_elev, cruise_alt, start_d_nm, profile, headwind,
            );
            let desc2 = integrate_vertical(arr_elev, peak, profile.desc_fpm, headwind, desc_tas);
            let climb2 = integrate_climb(start_alt, peak, profile, headwind);
            for &(d, a, t) in &desc2 {
                samples.push((d, a, t));
            }
            for &(cd, a, t) in climb2.iter().rev() {
                samples.push((start_d_nm - cd, a, t));
            }
        } else {
            for &(d, a, t) in &desc {
                samples.push((d, a, t));
            }
            samples.push((toc_d, cruise_alt, cruise_tas_at(cruise_alt))); // cruise start
            for &(cd, a, t) in climb.iter().rev() {
                samples.push((start_d_nm - cd, a, t));
            }
        }
        // Span the whole route, then sort & de-duplicate in `d`.
        samples.push((
            start_d_nm.max(desc_dist),
            start_alt,
            cruise_tas_at(start_alt),
        ));
        samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        samples.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);

        Self { headwind, samples }
    }

    /// Interpolate a sample field (`.1` altitude or `.2` TAS) at distance-to-destination `d`.
    fn interp(&self, d_nm: f64, field: impl Fn(&(f64, f64, f64)) -> f64) -> f64 {
        let d = d_nm.max(0.0);
        match self
            .samples
            .binary_search_by(|s| s.0.partial_cmp(&d).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => field(&self.samples[i]),
            Err(0) => self.samples.first().map(&field).unwrap_or(0.0),
            Err(i) if i >= self.samples.len() => self.samples.last().map(&field).unwrap_or(0.0),
            Err(i) => {
                let (s0, s1) = (&self.samples[i - 1], &self.samples[i]);
                let t = if (s1.0 - s0.0).abs() < 1e-9 {
                    0.0
                } else {
                    (d - s0.0) / (s1.0 - s0.0)
                };
                field(s0) + t * (field(s1) - field(s0))
            }
        }
    }

    /// Altitude (ft) at a distance-to-destination `d`.
    pub fn alt_at(&self, d_nm: f64) -> f64 {
        self.interp(d_nm, |s| s.1)
    }

    /// Predicted ground speed (kt) at a distance-to-destination `d` — the phase-appropriate TAS
    /// with wind applied, same as [`Self::time_between`] uses internally.
    pub fn ground_speed_at(&self, d_nm: f64) -> f64 {
        effective_gs(self.interp(d_nm, |s| s.2), self.headwind)
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
            let tas = self.interp(mid, |s| s.2);
            let gs = effective_gs(tas, self.headwind).max(GS_FLOOR_KT);
            secs += step / gs * 3600.0;
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
}

/// Integrate a climb/descent leg from `lo_alt` to `hi_alt` at a fixed vertical rate, returning
/// `(horizontal_distance_from_lo, altitude, true_airspeed)` samples. `speed_at(alt)` is the true
/// airspeed there; groundspeed for the distance is wind-corrected. Used for the descent leg.
fn integrate_vertical(
    lo_alt: f64,
    hi_alt: f64,
    fpm: f64,
    headwind: Option<f64>,
    speed_at: impl Fn(f64) -> f64,
) -> Vec<(f64, f64, f64)> {
    let mut out = vec![(0.0, lo_alt, speed_at(lo_alt))];
    if hi_alt <= lo_alt || fpm <= 0.0 {
        return out;
    }
    let mut alt = lo_alt;
    let mut dist = 0.0;
    while alt < hi_alt {
        let top = (alt + VERT_STEP_FT).min(hi_alt);
        let mid = (alt + top) / 2.0;
        let climb_sec = (top - alt) / fpm * 60.0;
        let tas = speed_at(mid);
        let gs = effective_gs(tas, headwind).max(GS_FLOOR_KT);
        dist += gs * climb_sec / 3600.0;
        alt = top;
        out.push((dist, alt, speed_at(alt)));
    }
    out
}

/// Integrate a climb from `lo_alt` to `hi_alt` with the profile's per-band climb rates
/// (`climb_fpm_lo` below 10,000 ft, `climb_fpm_hi` above), returning `(distance, altitude, tas)`.
fn integrate_climb(
    lo_alt: f64,
    hi_alt: f64,
    profile: &AircraftProfile,
    headwind: Option<f64>,
) -> Vec<(f64, f64, f64)> {
    let climb_tas = |alt: f64| {
        phase_tas(
            profile.climb_ias_lo,
            profile.climb_ias_hi,
            profile.climb_mach,
            alt,
        )
    };
    let mut out = vec![(0.0, lo_alt, climb_tas(lo_alt))];
    if hi_alt <= lo_alt {
        return out;
    }
    let mut alt = lo_alt;
    let mut dist = 0.0;
    while alt < hi_alt {
        let top = (alt + VERT_STEP_FT).min(hi_alt);
        let mid = (alt + top) / 2.0;
        let fpm = if mid < 10_000.0 {
            profile.climb_fpm_lo
        } else {
            profile.climb_fpm_hi
        }
        .max(1.0);
        let climb_sec = (top - alt) / fpm * 60.0;
        let tas = phase_tas(
            profile.climb_ias_lo,
            profile.climb_ias_hi,
            profile.climb_mach,
            mid,
        );
        let gs = effective_gs(tas, headwind).max(GS_FLOOR_KT);
        dist += gs * climb_sec / 3600.0;
        alt = top;
        out.push((dist, alt, climb_tas(alt)));
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
    profile: &AircraftProfile,
    headwind: Option<f64>,
) -> f64 {
    let dist_for = |peak: f64| -> f64 {
        let c = integrate_climb(start_alt, peak, profile, headwind)
            .last()
            .map(|&(d, ..)| d)
            .unwrap_or(0.0);
        let d = integrate_vertical(arr_elev, peak, profile.desc_fpm, headwind, |alt| {
            phase_tas(
                profile.desc_ias_lo,
                profile.desc_ias_hi,
                profile.desc_mach,
                alt,
            )
        })
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
        let hw = None;
        let cessna = VerticalProfile::build(0.0, 200.0, 0.0, 12000.0, 110.0, &c172(), hw);
        let heavy = VerticalProfile::build(0.0, 200.0, 0.0, 37000.0, 490.0, &b77w(), hw);
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
        let vp = VerticalProfile::build(35000.0, 300.0, 0.0, 35000.0, 480.0, &p, None);
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
        let vp = VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 110.0, &c172(), None);
        let peak = (0..=300).map(|d| vp.alt_at(d as f64)).fold(0.0, f64::max);
        assert!(
            peak <= 14000.5,
            "cruise capped at the 14k ceiling, peak {peak}"
        );
    }

    // ---- distance_after: time_between's inverse, for #226's forward prediction scrubber ----

    #[test]
    fn distance_after_round_trips_through_time_between() {
        let vp = VerticalProfile::build(35000.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), None);
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
        let vp = VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), None);
        assert!((vp.distance_after(300.0, 0.0) - 300.0).abs() < 1e-6);
    }

    #[test]
    fn distance_after_clamps_to_zero_once_the_flight_would_have_landed() {
        let vp = VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), None);
        let total = vp.time_between(300.0, 0.0);
        assert_eq!(vp.distance_after(300.0, total), 0.0);
        assert_eq!(vp.distance_after(300.0, total + 3600.0), 0.0);
    }

    #[test]
    fn short_hop_never_reaches_cruise() {
        // 40 nm hop: a heavy can't climb to FL370 and back down in the distance, so the modeled
        // peak altitude stays well under the requested cruise (the triangle case).
        let vp = VerticalProfile::build(0.0, 40.0, 0.0, 37000.0, 490.0, &b77w(), None);
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
        let still = VerticalProfile::build(35000.0, 400.0, 0.0, 35000.0, 460.0, &b77w(), None);
        let hw = VerticalProfile::build(35000.0, 400.0, 0.0, 35000.0, 460.0, &b77w(), Some(90.0));
        assert!(hw.time_between(400.0, 0.0) > still.time_between(400.0, 0.0));
    }

    #[test]
    fn ground_speed_at_reflects_headwind_and_matches_alt_at_s_own_phase() {
        // Same cruise-altitude comparison as `headwind_slows_the_vertical_model_too`, but on the
        // per-point speed accessor the #225 per-fix debug table uses directly.
        let still = VerticalProfile::build(35000.0, 400.0, 0.0, 35000.0, 460.0, &b77w(), None);
        let hw = VerticalProfile::build(35000.0, 400.0, 0.0, 35000.0, 460.0, &b77w(), Some(90.0));
        assert!(hw.ground_speed_at(200.0) < still.ground_speed_at(200.0));

        // Near the field on a descent, ground speed should be well below the cruise TAS.
        let descending = VerticalProfile::build(35000.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), None);
        assert!(
            descending.ground_speed_at(5.0) < 480.0,
            "expected a slower speed close to the field, got {}",
            descending.ground_speed_at(5.0)
        );
    }

    #[test]
    fn default_profile_climb_still_costs_time_over_cruise() {
        // Sanity that the Default profile behaves like a climbing jet: a surface departure over
        // 300 nm takes longer than the same distance flown entirely at cruise.
        let p = AircraftProfile::default();
        let dep = VerticalProfile::build(0.0, 300.0, 0.0, 35000.0, 450.0, &p, None);
        let climb_time = dep.time_between(300.0, 0.0);
        let pure_cruise = 300.0 / 450.0 * 3600.0;
        assert!(
            climb_time > pure_cruise,
            "climb {climb_time} should exceed pure cruise {pure_cruise}"
        );
    }
}
