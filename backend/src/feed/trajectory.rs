//! Shared trajectory / ETA model: the single predictor used by every surface that estimates where
//! connected aircraft will be at a given time (FCA metering, airport-flow demand, runway ETE), so those
//! surfaces stay consistent. It ports vTBFM's trajectory model (#985); `docs/architecture/prediction.md`
//! describes it and where it differs.
//!
//! A flight is flown forward from where it is, in short steps: each step picks climb, cruise or descent
//! from the distance to go and the altitude, takes its speed from the type's [`Performance`], and the
//! wind and temperature from a [`RouteWind`] sampled at that point ([`crate::feed::winds`]). It fails
//! safe to still air when winds are unavailable.

use super::performance::{EramTable, Performance};

/// Lower bound (kt) on a believable filed cruise TAS: below it, nothing was filed or it was a Mach number.
pub const MIN_PLAUSIBLE_TAS: f64 = 60.0;

/// Upper bound on a *believable* filed cruise TAS (kt). Pilots sometimes file garbage (e.g. `4800`),
/// and since the ETA clamps groundspeed only *relative* to TAS, an absurd value would collapse the
/// transit time and shoot the aircraft to the front of the sequence. Anything above this is treated
/// as unfiled and replaced with the altitude-based default. Comfortably above any real airliner.
pub const MAX_PLAUSIBLE_TAS: f64 = 700.0;

/// Upper bound (ft) on a believable altitude, filed or reported, above any aircraft on the network. ERAM
/// gives no ceiling, so this is what keeps a junk filing from setting the length of a climb.
const MAX_PLAUSIBLE_ALT_FT: f64 = 100_000.0;

/// The cruise altitude (ft) a flight with no readable filing is timed at, if its type cruises that high.
const UNFILED_CRUISE_FT: f64 = 35_000.0;

/// The filed cruise altitude in feet, or `None` when it is unfiled or unreadable (#813).
///
/// Reads the **first** run of digits, so a step climb filed `"FL350/370"` is FL350 rather than the
/// 350,370 ft that concatenating every digit gives. A value ≤ 600 is a flight level (×100); anything
/// that comes out under 1,000 ft (`"VFR"`, `"0"`, `"8"`) is unknown. The FCA altitude band and the
/// timing model both read the filing through this one parser.
pub fn filed_altitude_ft(s: &str) -> Option<f64> {
    let digits: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    let mut n: f64 = digits.parse().ok()?;
    if n <= 600.0 {
        n *= 100.0;
    }
    (n >= 1000.0).then_some(n)
}

/// The cruise altitude a filing is shown as: [`filed_altitude_ft`], or 35,000 ft when it is unknown.
pub fn parse_alt_ft(s: &str) -> f64 {
    filed_altitude_ft(s).unwrap_or(UNFILED_CRUISE_FT)
}

/// Whether a filed TAS (kt) is worth reading: not missing, a Mach number in the wrong field, or garbage.
pub(crate) fn believable_tas(tas: f64) -> bool {
    (MIN_PLAUSIBLE_TAS..=MAX_PLAUSIBLE_TAS).contains(&tas)
}

/// Filed TAS (kt) with a sane fallback estimated from cruise altitude. A filed value outside the
/// believable band (too slow, or garbage like `4800`) is discarded for the altitude-based default.
pub fn tas_or_default(tas: f64, cruise_alt_ft: f64) -> f64 {
    if believable_tas(tas) {
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

/// The least and the most of a true airspeed that a groundspeed can be, whatever the wind reads.
const MIN_GS_FRACTION: f64 = 0.4;
const MAX_GS_FRACTION: f64 = 1.6;

/// Groundspeed from TAS and headwind (`+` = headwind, `-` = tailwind), clamped to a sane
/// band so a bad wind reading can't produce absurd speeds. Still air when `headwind` is None.
pub fn effective_gs(tas: f64, headwind: Option<f64>) -> f64 {
    match headwind {
        None => tas,
        Some(hw) => {
            // Order the bounds so a nonsensical negative TAS can't invert the clamp range.
            let (lo, hi) = if tas >= 0.0 {
                (tas * MIN_GS_FRACTION, tas * MAX_GS_FRACTION)
            } else {
                (tas * MAX_GS_FRACTION, tas * MIN_GS_FRACTION)
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
    match profile.cruise_tas {
        Some(cap) if believable_tas(filed_tas) => filed_tas.min(cap),
        Some(cap) => cap,
        None => tas_or_default(filed_tas, cruise_alt_ft),
    }
}

/// Standard-atmosphere constants (ISA, sea level).
const ISA_T0_K: f64 = 288.15; // sea-level temperature, K
const ISA_LAPSE_K_PER_FT: f64 = 0.0019812; // 1.98 K per 1000 ft, troposphere
const ISA_TROPOPAUSE_FT: f64 = 36089.0;
const ISA_TROPOPAUSE_T_K: f64 = 216.65; // isothermal above the tropopause
const SPEED_OF_SOUND_SL_KT: f64 = 661.4788; // a0 at ISA sea level, knots

/// ISA temperature (K) at a pressure altitude.
pub(crate) fn isa_temp_k(alt_ft: f64) -> f64 {
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

/// Calibrated airspeed (kt) for a true airspeed at altitude: the inverse of [`ias_to_tas`].
pub fn tas_to_cas(tas_kt: f64, alt_ft: f64) -> f64 {
    tas_kt * isa_density_ratio(alt_ft.max(0.0)).max(1e-3).sqrt()
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
/// climb/cruise numbers (250/290 kt, 2000/1500 fpm, no Mach) plus a plain jet descent.
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

/// The aircraft performance a flight is picked from: a stored type row → ERAM's record of the type →
/// engine family row (`jet` / `turboprop` / `piston`, from [`crate::feed::flow::engine_family`]) →
/// wake-class row (`L/M/H/J`) → the global default row. Rows are built from the DB by
/// [`crate::repos::aircraft_profiles`]; the whole is cached in `AppState`. `Default` holds the legacy
/// default row and nothing else; [`Self::built_in`] is the fallback before the rows have loaded.
#[derive(Debug, Clone, Default)]
pub struct ProfileTable {
    default: AircraftProfile,
    by_wake: std::collections::HashMap<String, AircraftProfile>,
    by_family: std::collections::HashMap<String, AircraftProfile>,
    by_type: std::collections::HashMap<String, AircraftProfile>,
    eram: Option<&'static EramTable>,
}

/// The tier [`ProfileTable`] matched, so `resolve` and `resolve_label` read one decision.
enum Tier {
    Type(String),
    Eram(String),
    Family(&'static str),
    Wake(String),
    Default,
}

impl ProfileTable {
    /// `by_wake` and `by_type` are keyed uppercase, `by_family` lowercase, as
    /// `repos::aircraft_profiles::StoredTiers` stores them.
    pub fn new(
        default: AircraftProfile,
        by_wake: std::collections::HashMap<String, AircraftProfile>,
        by_family: std::collections::HashMap<String, AircraftProfile>,
        by_type: std::collections::HashMap<String, AircraftProfile>,
        eram: Option<&'static EramTable>,
    ) -> Self {
        Self {
            default,
            by_wake,
            by_family,
            by_type,
            eram,
        }
    }

    /// The bundled ERAM table over the legacy default row: what flies until the stored rows load.
    pub fn built_in() -> Self {
        Self {
            eram: Some(EramTable::bundled()),
            ..Self::default()
        }
    }

    fn tier(&self, ty: &str, wake: &str) -> (Tier, Performance<'_>) {
        let ty = ty.to_ascii_uppercase();
        if let Some(p) = self.by_type.get(&ty) {
            return (Tier::Type(ty), Performance::Row(p));
        }
        if let Some(r) = self.eram.and_then(|e| e.get(&ty)) {
            return (Tier::Eram(ty), Performance::Eram(r));
        }
        let (tier, p) = self.row_below_type(&ty, wake);
        (tier, Performance::Row(p))
    }

    /// The row an uppercase type falls to when no row names it: family, wake class, default.
    fn row_below_type(&self, ty: &str, wake: &str) -> (Tier, &AircraftProfile) {
        let family = crate::feed::flow::engine_family(ty);
        if let Some((f, p)) = family.and_then(|f| self.by_family.get(f).map(|p| (f, p))) {
            return (Tier::Family(f), p);
        }
        let wake = wake.to_ascii_uppercase();
        if let Some(p) = self.by_wake.get(&wake) {
            return (Tier::Wake(wake), p);
        }
        (Tier::Default, &self.default)
    }

    /// The performance an aircraft flies: its type row → ERAM → family row → wake row → default row.
    pub fn resolve(&self, ty: &str, wake: &str) -> Performance<'_> {
        self.tier(ty, wake).1
    }

    /// The row an aircraft would fly with no ERAM table: what a measured row for its type starts from
    /// (`profile_builder::backtest`).
    pub fn row(&self, ty: &str, wake: &str) -> &AircraftProfile {
        let ty = ty.to_ascii_uppercase();
        match self.by_type.get(&ty) {
            Some(p) => p,
            None => self.row_below_type(&ty, wake).1,
        }
    }

    /// A label for which tier `resolve` matched, for the debug view: `type:<T>`, `eram:<T>`,
    /// `family:<F>`, `wake:<W>`, or `default`.
    pub fn resolve_label(&self, ty: &str, wake: &str) -> String {
        match self.tier(ty, wake).0 {
            Tier::Type(t) => format!("type:{t}"),
            Tier::Eram(t) => format!("eram:{t}"),
            Tier::Family(f) => format!("family:{f}"),
            Tier::Wake(w) => format!("wake:{w}"),
            Tier::Default => "default".to_string(),
        }
    }
}

/// The governing true airspeed for a climb/descent phase at `alt`: the IAS-schedule TAS,
/// held down to the Mach number's TAS when one is set (the SimBrief crossover — Mach wins high,
/// IAS wins low). `ias` is the schedule speed for the altitude band; `mach` the phase Mach, flown in
/// air at `temp_k`.
pub(crate) fn phase_tas(
    ias_lo: f64,
    ias_hi: f64,
    mach: Option<f64>,
    alt_ft: f64,
    temp_k: f64,
) -> f64 {
    let ias = if alt_ft < 10_000.0 { ias_lo } else { ias_hi };
    let tas_ias = ias_to_tas(ias, alt_ft);
    match mach {
        Some(m) => tas_ias.min(mach_to_tas_at(m, temp_k)),
        None => tas_ias,
    }
}

/// Groundspeed floor (kt), so no step stalls.
const GS_FLOOR_KT: f64 = 60.0;
/// Length (s) of one integration step; the last one is cut short at the field.
const STEP_SEC: f64 = 15.0;
/// A route longer than this (nm), over once around the earth, is flown in steps longer in proportion, so
/// a junk route cannot set how many there are.
const MAX_STEPPED_NM: f64 = 25_000.0;
/// Descent gradient: 1,000 ft for every 3 nm to the field.
const DESCENT_FT_PER_NM: f64 = 1000.0 / 3.0;
/// Descent speeds start this far (nm) outside the type's own descent distance.
const DESCENT_MARGIN_NM: f64 = 10.0;
/// A flight this close (ft) below its cruise level is at cruise.
const CRUISE_BAND_FT: f64 = 500.0;
/// A flight below its filed altitude is level there once it is farther from its departure than this
/// many times its type's climb distance.
const LEVEL_CLIMB_DISTANCES: f64 = 1.5;
/// A descending flight this much (kt CAS) slower than predicted is flying an assigned speed.
const ASSIGNED_SPEED_DEADBAND_KT: f64 = 15.0;
/// The anchoring scale is clamped to ±this fraction, so one noisy groundspeed can't throw the
/// trajectory.
const ANCHOR_MAX_BIAS: f64 = 0.15;

/// Where a flight starts: its altitude, its distance along the route to the field, and what is known
/// of one already flying.
pub struct FlightStart {
    pub alt_ft: f64,
    pub to_go_nm: f64,
    /// None on the ground.
    pub airborne: Option<Airborne>,
}

impl FlightStart {
    /// Its altitude, held to a believable one.
    fn altitude(&self) -> f64 {
        self.alt_ft.clamp(0.0, MAX_PLAUSIBLE_ALT_FT)
    }
}

/// What an airborne flight is doing.
#[derive(Debug, Clone, Copy)]
pub struct Airborne {
    pub gs_kt: f64,
    /// The headwind (kt) along its heading at its position and altitude; None in still air.
    pub headwind_kt: Option<f64>,
    /// Straight-line distance (nm) from its departure field; None when that field is unknown.
    pub from_departure_nm: Option<f64>,
}

/// What a flight filed and where it lands.
pub struct Filed {
    /// None when no altitude was readable.
    pub cruise_ft: Option<f64>,
    /// The filed cruise true airspeed (kt) as filed; 0 when none.
    pub tas_kt: f64,
    pub arr_elev_ft: f64,
}

impl Filed {
    /// The altitude (ft) a row reads the filing at.
    fn altitude_ft(&self) -> f64 {
        self.cruise_ft.unwrap_or(UNFILED_CRUISE_FT)
    }
}

/// A point the flight passes: its distance to the field, its altitude there, the time from the start,
/// and the groundspeed and headwind of the step it flies **from** here (the last point repeats the last
/// step's).
#[derive(Debug, Clone, Copy)]
struct Sample {
    d_nm: f64,
    alt_ft: f64,
    sec: f64,
    gs_kt: f64,
    headwind_kt: f64,
}

/// A flight flown forward from where it is to the field, in steps of [`STEP_SEC`]. Built once per flight,
/// then queried by distance to the field (`d`): `d = 0` is the field, increasing back along the route.
///
/// Every query reads the one track, so a time, the speed it was flown at and the altitude there always
/// describe the same flight.
pub struct VerticalProfile {
    /// The wind carries no level anywhere on the route.
    still: bool,
    cruise_tas: f64,
    /// In increasing `d`; never empty.
    samples: Vec<Sample>,
}

impl VerticalProfile {
    /// Fly `start` to the field. `wind` is the route's wind by distance to the field, on the same `d`
    /// axis as `start.to_go_nm`.
    pub fn build(start: &FlightStart, filed: &Filed, perf: Performance, wind: RouteWind) -> Self {
        let plan = Plan::new(start, filed, perf, &wind);
        let mut samples = plan.fly();
        samples.reverse();
        let (_, temp) = wind.air(start.to_go_nm, plan.level_ft);
        Self {
            still: wind.is_still(),
            cruise_tas: perf.cruise_in(plan.cruise_tas, plan.level_ft, temp),
            samples,
        }
    }

    /// The cruise true airspeed (kt) the flight is predicted at from where it starts.
    pub fn cruise_tas(&self) -> f64 {
        self.cruise_tas
    }

    /// Index of the sample the flight at `d` was last at: the nearest one at or beyond it.
    fn behind(&self, d_nm: f64) -> usize {
        self.samples
            .partition_point(|s| s.d_nm < d_nm)
            .min(self.samples.len() - 1)
    }

    /// Altitude (ft) at a distance `d` from the field.
    pub fn alt_at(&self, d_nm: f64) -> f64 {
        let i = self.behind(d_nm);
        let from = self.samples[i];
        let Some(to) = i.checked_sub(1).map(|j| self.samples[j]) else {
            return from.alt_ft;
        };
        let flown = (from.d_nm - d_nm).max(0.0) / (from.d_nm - to.d_nm);
        from.alt_ft + flown * (to.alt_ft - from.alt_ft)
    }

    /// Predicted groundspeed (kt) at `d`: the speed of the step that passes it, which is the speed
    /// [`Self::time_between`] flew there (#361).
    pub fn ground_speed_at(&self, d_nm: f64) -> f64 {
        self.samples[self.behind(d_nm)].gs_kt
    }

    /// Seconds from the start to `d`.
    fn sec_to(&self, d_nm: f64) -> f64 {
        let s = self.samples[self.behind(d_nm)];
        s.sec + (s.d_nm - d_nm.max(0.0)).max(0.0) / s.gs_kt * 3600.0
    }

    /// Seconds to fly from distance-to-destination `from_d` forward to `to_d` (`to_d < from_d`).
    pub fn time_between(&self, from_d: f64, to_d: f64) -> f64 {
        (self.sec_to(to_d) - self.sec_to(from_d)).abs()
    }

    /// Inverse of [`Self::time_between`]: the distance to the field `elapsed_sec` after `from_d` (#226's
    /// projection). `0.0`, landed, once `elapsed_sec` reaches the flying time left.
    pub fn distance_after(&self, from_d: f64, elapsed_sec: f64) -> f64 {
        if elapsed_sec <= 0.0 {
            return from_d;
        }
        let at = self.sec_to(from_d) + elapsed_sec;
        if at >= self.samples[0].sec {
            return 0.0;
        }
        // Samples run from the field back, so elapsed time falls along them.
        let s = self.samples[self.samples.partition_point(|s| s.sec > at)];
        (s.d_nm - s.gs_kt * (at - s.sec) / 3600.0).max(0.0)
    }

    /// The distance-weighted mean headwind (kt) flown between `from_d` and `to_d`; a stretch the field
    /// does not cover counts as 0. A summary for the debug view. None in still air.
    pub fn mean_headwind(&self, from_d: f64, to_d: f64) -> Option<f64> {
        let (hi, lo) = (from_d.max(to_d), from_d.min(to_d));
        if self.still {
            return None;
        }
        let (mut sum, mut nm) = (0.0, 0.0);
        for step in self.samples.windows(2) {
            let flown = step[1].d_nm.min(hi) - step[0].d_nm.max(lo);
            if flown > 0.0 {
                sum += flown * step[1].headwind_kt;
                nm += flown;
            }
        }
        (nm > 0.0).then_some(sum / nm)
    }
}

/// Everything a flight's steps are decided from, settled once before the first step.
struct Plan<'a> {
    start: &'a FlightStart,
    perf: Performance<'a>,
    wind: &'a RouteWind,
    elev_ft: f64,
    /// The altitude the flight cruises at.
    cruise_ft: f64,
    /// The level it stops climbing at: [`Self::cruise_ft`], or its present altitude when it is level
    /// below that.
    level_ft: f64,
    cruise_tas: f64,
    /// Distance to the field (nm) inside which the flight descends.
    descent_start_nm: f64,
    /// The flight climbs until it meets the descent path, wherever that is.
    climbing: bool,
    /// Multiplier on every groundspeed.
    gs_scale: f64,
    /// A calibrated airspeed (kt) the flight is held to.
    cas_ceiling: Option<f64>,
}

impl<'a> Plan<'a> {
    fn new(
        start: &'a FlightStart,
        filed: &Filed,
        perf: Performance<'a>,
        wind: &'a RouteWind,
    ) -> Self {
        let elev_ft = filed.arr_elev_ft.max(0.0);
        let cruise_ft = cruise_altitude(filed, perf, start.altitude().max(elev_ft));
        let filed_cruise_tas = perf
            .cruise_tas(filed.tas_kt, filed.altitude_ft(), cruise_ft)
            .max(GS_FLOOR_KT);
        let descent_start_nm = descent_start_nm(perf, cruise_ft, elev_ft, filed_cruise_tas);
        let mut plan = Self {
            start,
            perf,
            wind,
            elev_ft,
            cruise_ft,
            level_ft: cruise_ft,
            cruise_tas: filed_cruise_tas,
            descent_start_nm,
            climbing: false,
            gs_scale: 1.0,
            cas_ceiling: None,
        };
        if plan.is_level_below_cruise() {
            plan.level_ft = plan.alt0();
            plan.cruise_tas = perf
                .cruise_tas(filed.tas_kt, filed.altitude_ft(), plan.level_ft)
                .max(GS_FLOOR_KT);
        }
        plan.climbing = plan.nearer_departure()
            && plan.below_level(plan.alt0())
            && plan.alt0() < plan.path_ft(start.to_go_nm);
        plan.gs_scale = plan.anchor_scale();
        plan.hold_assigned_speed();
        plan
    }

    fn alt0(&self) -> f64 {
        self.start.altitude()
    }

    fn below_level(&self, alt_ft: f64) -> bool {
        alt_ft < self.level_ft - CRUISE_BAND_FT
    }

    /// The descent path's altitude `d` from the field.
    fn path_ft(&self, d_nm: f64) -> f64 {
        self.elev_ft + DESCENT_FT_PER_NM * d_nm
    }

    /// On the ground, or still nearer its departure than its destination.
    fn nearer_departure(&self) -> bool {
        self.start
            .airborne
            .as_ref()
            .is_none_or(|a| a.from_departure_nm.is_some_and(|d| d < self.start.to_go_nm))
    }

    /// One VATSIM position carries no vertical rate, so "level" is read from where the flight is: below
    /// its cruise altitude, outside its descent, and well past where its type's climb would have ended.
    fn is_level_below_cruise(&self) -> bool {
        let from_departure = self
            .start
            .airborne
            .as_ref()
            .and_then(|a| a.from_departure_nm);
        let Some(from_departure) = from_departure else {
            return false;
        };
        self.start.to_go_nm > self.descent_start_nm
            && self.below_level(self.alt0())
            && from_departure > LEVEL_CLIMB_DISTANCES * self.perf.climb_distance_nm(self.cruise_ft)
    }

    /// The observed groundspeed over the predicted one, for a flight established at its level or above
    /// it; 1 for any other.
    fn anchor_scale(&self) -> f64 {
        let Some(a) = &self.start.airborne else {
            return 1.0;
        };
        let alt = self.alt0();
        if self.below_level(alt) {
            return 1.0;
        }
        let (_, temp) = self.wind.air(self.start.to_go_nm, alt);
        let tas = self.perf.cruise_in(self.cruise_tas, alt, temp);
        let predicted = effective_gs(tas, a.headwind_kt).max(GS_FLOOR_KT);
        (a.gs_kt / predicted).clamp(1.0 - ANCHOR_MAX_BIAS, 1.0 + ANCHOR_MAX_BIAS)
    }

    /// Hold a flight that starts inside its descent, slower than predicted by more than the deadband, to
    /// the speed it is flying. It replaces the anchoring scale, which would slow it twice. One with no known
    /// departure may be climbing out of it, so it is not held.
    fn hold_assigned_speed(&mut self) {
        let known = |a: &&Airborne| a.from_departure_nm.is_some();
        let Some(a) = self.start.airborne.as_ref().filter(known) else {
            return;
        };
        if self.climbing || self.start.to_go_nm > self.descent_start_nm {
            return;
        }
        let alt = self.alt0();
        let (_, temp) = self.wind.air(self.start.to_go_nm, alt);
        let predicted = self.perf.descent_tas(alt, temp, self.cruise_tas) * self.gs_scale;
        let observed = tas_to_cas(a.gs_kt + a.headwind_kt.unwrap_or(0.0), alt);
        if observed < tas_to_cas(predicted, alt) - ASSIGNED_SPEED_DEADBAND_KT {
            self.cas_ceiling = Some(observed);
            self.gs_scale = 1.0;
        }
    }

    /// The flight's points from its start to the field.
    fn fly(&self) -> Vec<Sample> {
        let mut out = Vec::new();
        let (mut d, mut alt, mut sec) = (self.start.to_go_nm.max(0.0), self.alt0(), 0.0);
        let full_step_sec = STEP_SEC * (d / MAX_STEPPED_NM).max(1.0);
        let mut climbing = self.climbing;
        let (mut gs, mut headwind) = (self.cruise_tas, 0.0);
        while d > 0.0 {
            climbing &= alt < self.level_ft && alt < self.path_ft(d);
            let phase = self.phase(d, alt, climbing);
            let (hw, temp) = self.wind.air(d, alt);
            let (tas, fpm) = self.speed(phase, alt, temp);
            gs = (effective_gs(tas, hw) * self.gs_scale).max(GS_FLOOR_KT);
            headwind = hw.unwrap_or(0.0);
            out.push(Sample {
                d_nm: d,
                alt_ft: alt,
                sec,
                gs_kt: gs,
                headwind_kt: headwind,
            });
            let step_nm = (gs * full_step_sec / 3600.0).min(d);
            let step_sec = step_nm / gs * 3600.0;
            d -= step_nm;
            sec += step_sec;
            alt = match phase {
                Phase::Climb => (alt + fpm * step_sec / 60.0)
                    .min(self.cruise_ft)
                    .min(self.path_ft(d)),
                // The last of the climb: a flight in the cruise band goes on to its level.
                Phase::Cruise => alt.max(self.level_ft.min(self.path_ft(d))),
                Phase::Descent => alt.min(self.path_ft(d)),
            };
        }
        out.push(Sample {
            d_nm: 0.0,
            alt_ft: alt,
            sec,
            gs_kt: gs,
            headwind_kt: headwind,
        });
        out
    }

    fn phase(&self, d_nm: f64, alt_ft: f64, climbing: bool) -> Phase {
        if d_nm <= self.descent_start_nm && !climbing {
            Phase::Descent
        } else if self.below_level(alt_ft) {
            Phase::Climb
        } else {
            Phase::Cruise
        }
    }

    /// The true airspeed (kt) of a step in `phase` at `alt_ft`, and the climb rate (fpm) of a climb.
    fn speed(&self, phase: Phase, alt_ft: f64, temp_k: Option<f64>) -> (f64, f64) {
        let (tas, fpm) = match phase {
            Phase::Climb => self.perf.climb(alt_ft, temp_k),
            Phase::Cruise => (self.perf.cruise_in(self.cruise_tas, alt_ft, temp_k), 0.0),
            Phase::Descent => (self.perf.descent_tas(alt_ft, temp_k, self.cruise_tas), 0.0),
        };
        match self.cas_ceiling {
            Some(cas) => (tas.min(ias_to_tas(cas, alt_ft)), fpm),
            None => (tas, fpm),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Climb,
    Cruise,
    Descent,
}

/// The altitude a flight cruises at: its filing, held to the type's ceiling, never below `at_least_ft`.
/// A type with no ceiling reads a filing above [`MAX_PLAUSIBLE_ALT_FT`] as unfiled, and flies an unfiled one
/// no higher than it has a cruise speed for.
fn cruise_altitude(filed: &Filed, perf: Performance, at_least_ft: f64) -> f64 {
    let believed = filed.cruise_ft.filter(|ft| *ft <= MAX_PLAUSIBLE_ALT_FT);
    let cruise_ft = match perf.ceiling_ft() {
        Some(ceiling) => filed.altitude_ft().min(ceiling),
        None => believed.unwrap_or_else(|| UNFILED_CRUISE_FT.min(perf.cruise_top_ft())),
    };
    cruise_ft.max(at_least_ft)
}

/// Distance to the field (nm) at which descent speeds start: the type's own descent plus its margin, and
/// never inside the point where the descent path reaches cruise.
fn descent_start_nm(perf: Performance, cruise_ft: f64, elev_ft: f64, cruise_tas: f64) -> f64 {
    let own = perf.descent_distance_nm(cruise_ft, elev_ft, cruise_tas) + DESCENT_MARGIN_NM;
    own.max((cruise_ft - elev_ft) / DESCENT_FT_PER_NM)
}

/// The groundspeed (kt) an aircraft flies `d_nm` from the field on its descent, whatever it is doing now:
/// on the descent path at its descent speed, or at its cruise speed when `d_nm` lies above the top of
/// descent. There is no anchoring and no assigned speed.
pub fn descent_ground_speed_at(
    d_nm: f64,
    filed: &Filed,
    perf: Performance,
    wind: &RouteWind,
) -> f64 {
    let elev = filed.arr_elev_ft.max(0.0);
    let cruise_ft = cruise_altitude(filed, perf, elev);
    let cruise_tas = perf
        .cruise_tas(filed.tas_kt, filed.altitude_ft(), cruise_ft)
        .max(GS_FLOOR_KT);
    let alt = (elev + DESCENT_FT_PER_NM * d_nm).min(cruise_ft);
    let (hw, temp) = wind.air(d_nm, alt);
    let tas = if d_nm <= descent_start_nm(perf, cruise_ft, elev, cruise_tas) {
        perf.descent_tas(alt, temp, cruise_tas)
    } else {
        perf.cruise_in(cruise_tas, alt, temp)
    };
    effective_gs(tas, hw).max(GS_FLOOR_KT)
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

    /// A step climb reads its first level, not every digit run concatenated (#813, from #838).
    #[test]
    fn a_step_climb_filing_reads_its_first_level() {
        assert_eq!(filed_altitude_ft("FL350/370"), Some(35000.0));
        assert_eq!(parse_alt_ft("FL350/370"), 35000.0);
        assert_eq!(filed_altitude_ft("35000/37000"), Some(35000.0));
        assert_eq!(filed_altitude_ft("A045"), Some(4500.0));
    }

    /// Unknown is unknown to the band and the model, and shown as 35,000 ft; a filing under 1,000 ft is
    /// unknown, at the boundary as well as below it.
    #[test]
    fn an_unreadable_filing_is_unknown_and_shown_as_35000_ft() {
        for s in ["", "VFR", "0", "8", "700"] {
            assert_eq!(filed_altitude_ft(s), None, "{s:?}");
            assert_eq!(parse_alt_ft(s), 35000.0, "{s:?}");
        }
        assert_eq!(filed_altitude_ft("10"), Some(1000.0));
        assert_eq!(filed_altitude_ft("600"), Some(60000.0));
        assert_eq!(filed_altitude_ft("1000"), Some(1000.0));
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

    /// `start_alt` at `start_d` from the field with no groundspeed read: a departure, or the raw profile
    /// of a flight already up there.
    fn flown(
        start_alt: f64,
        start_d: f64,
        arr_elev_ft: f64,
        cruise_ft: f64,
        tas_kt: f64,
        p: &AircraftProfile,
        wind: RouteWind,
    ) -> VerticalProfile {
        let start = FlightStart {
            alt_ft: start_alt,
            to_go_nm: start_d,
            airborne: None,
        };
        let filed = Filed {
            cruise_ft: Some(cruise_ft),
            tas_kt,
            arr_elev_ft,
        };
        VerticalProfile::build(&start, &filed, Performance::Row(p), wind)
    }

    /// A flight at FL350, `start_d` out, at `gs` along the route.
    fn anchored(
        start_d: f64,
        tas_kt: f64,
        p: &AircraftProfile,
        wind: RouteWind,
        gs: f64,
    ) -> VerticalProfile {
        let start = FlightStart {
            alt_ft: 35_000.0,
            to_go_nm: start_d,
            airborne: Some(Airborne {
                gs_kt: gs,
                headwind_kt: wind.headwind(start_d, 35_000.0),
                from_departure_nm: None,
            }),
        };
        let filed = Filed {
            cruise_ft: Some(35_000.0),
            tas_kt,
            arr_elev_ft: 0.0,
        };
        VerticalProfile::build(&start, &filed, Performance::Row(p), wind)
    }

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

    /// [`b77w`] with no cruise TAS of its own, so its Mach is what holds a 520 kt filing.
    fn mach_cruiser() -> AircraftProfile {
        AircraftProfile {
            cruise_tas: None,
            ..b77w()
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
        let cessna = flown(0.0, 200.0, 0.0, 12000.0, 110.0, &c172(), hw.clone());
        let heavy = flown(0.0, 200.0, 0.0, 37000.0, 490.0, &b77w(), hw.clone());
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
        let vp = flown(35000.0, 300.0, 0.0, 35000.0, 480.0, &p, RouteWind::still());
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
        let vp = flown(0.0, 300.0, 0.0, 35000.0, 110.0, &c172(), RouteWind::still());
        let peak = (0..=300).map(|d| vp.alt_at(d as f64)).fold(0.0, f64::max);
        assert!(
            peak <= 14000.5,
            "cruise capped at the 14k ceiling, peak {peak}"
        );
    }

    // ---- distance_after: time_between's inverse, for #226's forward prediction scrubber ----

    #[test]
    fn distance_after_round_trips_through_time_between() {
        let vp = flown(
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
        let vp = flown(0.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), RouteWind::still());
        assert!((vp.distance_after(300.0, 0.0) - 300.0).abs() < 1e-6);
    }

    #[test]
    fn distance_after_clamps_to_zero_once_the_flight_would_have_landed() {
        let vp = flown(0.0, 300.0, 0.0, 35000.0, 480.0, &b77w(), RouteWind::still());
        let total = vp.time_between(300.0, 0.0);
        assert_eq!(vp.distance_after(300.0, total), 0.0);
        assert_eq!(vp.distance_after(300.0, total + 3600.0), 0.0);
    }

    #[test]
    fn short_hop_never_reaches_cruise() {
        // 40 nm hop: a heavy can't climb to FL370 and back down in the distance, so the modeled
        // peak altitude stays well under the requested cruise (the triangle case).
        let vp = flown(0.0, 40.0, 0.0, 37000.0, 490.0, &b77w(), RouteWind::still());
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
        let still = flown(
            35000.0,
            400.0,
            0.0,
            35000.0,
            460.0,
            &b77w(),
            RouteWind::still(),
        );
        let hw = flown(
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
        let still = flown(
            35000.0,
            400.0,
            0.0,
            35000.0,
            460.0,
            &b77w(),
            RouteWind::still(),
        );
        let hw = flown(
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
        let descending = flown(
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
        let slow_with_headwind = flown(
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
        let dep = flown(0.0, 300.0, 0.0, 35000.0, 450.0, &p, RouteWind::still());
        let climb_time = dep.time_between(300.0, 0.0);
        let pure_cruise = 300.0 / 450.0 * 3600.0;
        assert!(
            climb_time > pure_cruise,
            "climb {climb_time} should exceed pure cruise {pure_cruise}"
        );
    }

    /// Regression (#355): anywhere the profile is still at cruise altitude, `ground_speed_at` must
    /// not read above the cruise groundspeed. The *descent* schedule's TAS at cruise altitude is higher
    /// (~521 kt for this profile at FL350 against a 440 kt cruise). FCA MIT spacing is sized
    /// from this number, so an un-capped read under-provisions every enroute crossing: a 20 MIT fix
    /// 150 nm out would get `20 / 508 * 3600 = 142 s`, which the aircraft flies at its real 440 kt
    /// for 17.3 nm, not 20.
    #[test]
    fn ground_speed_at_never_reads_above_cruise_while_still_at_cruise_altitude() {
        let profile = AircraftProfile::default();
        let cruise_tas = 440.0;
        let vp = flown(
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

    // ---- arrival-field elevation (#315) ----

    #[test]
    fn a_high_field_arrival_starts_its_descent_nearer_the_field() {
        // Same cruise and route into a sea-level field and into DEN (5,431 ft).
        let p = AircraftProfile::default();
        let sea = flown(35000.0, 300.0, 0.0, 35000.0, 450.0, &p, RouteWind::still());
        let den = flown(
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
        let from_sea_level = flown(0.0, 300.0, 5431.0, 35000.0, 450.0, &p, RouteWind::still());
        let from_field_height = flown(
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

    /// With no `desc_mach` the default profile's 290 KIAS descent schedule resolves to ~521 kt TAS at
    /// FL350, above the 440 kt cruise, and descent speeds start before the path leaves FL350.
    /// Removing the cruise cap from `Performance::descent_tas` fails here.
    #[test]
    fn the_cruise_leg_is_flat_at_cruise_speed() {
        let profile = AircraftProfile::default();
        let cruise_tas = 440.0;
        let vp = flown(
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
                (vp.ground_speed_at(d) - cruise_gs).abs() < 1.0,
                "d={d}: the cruise leg must be flat at {cruise_gs:.0} kt, got {:.1} kt",
                vp.ground_speed_at(d)
            );
        }
    }

    /// A hop too short to cruise climbs until it meets the descent path and turns down there, with no
    /// level-off between.
    #[test]
    fn a_short_hop_climbs_and_descends_to_one_peak() {
        let vp = flown(
            0.0,
            60.0,
            0.0,
            35_000.0,
            260.0,
            &AircraftProfile::default(),
            RouteWind::still(),
        );
        let step = 0.01;
        let alts: Vec<f64> = (0..=6_000).map(|i| vp.alt_at(i as f64 * step)).collect();
        let peak = alts.iter().copied().fold(f64::MIN, f64::max);
        assert!(peak < 30_000.0, "a short hop, peak {peak:.0} ft");
        let level_nm = alts.iter().filter(|&&a| a > peak - 1.0).count() as f64 * step;
        assert!(
            level_nm < 0.5,
            "the profile holds its {peak:.0} ft peak for {level_nm:.2} nm"
        );
    }

    /// The profile must never predict an aircraft flying faster than it cruises, at any point.
    /// This is the invariant the old `ground_speed_at` cap enforced as a workaround; now it holds
    /// in the samples themselves, so it is true for `time_between` (and therefore every ETA) too.
    #[test]
    fn the_profile_never_exceeds_cruise_speed() {
        let profile = AircraftProfile::default();
        let cruise_tas = 440.0;
        let route = 300.0;
        let vp = flown(
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
                vp.ground_speed_at(d) <= cruise_gs + 1.0,
                "d={d:.0}: predicted {:.1} kt exceeds the {cruise_gs:.0} kt cruise groundspeed",
                vp.ground_speed_at(d)
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
        let vp = flown(
            35_000.0,
            300.0,
            0.0,
            35_000.0,
            440.0,
            &profile,
            RouteWind::still(),
        );

        let near_field = vp.ground_speed_at(20.0);
        assert!(
            (250.0..320.0).contains(&near_field),
            "a fix 20 nm out should read the arrival descent speed, got {near_field:.1} kt"
        );
        assert!(
            vp.ground_speed_at(20.0) < vp.ground_speed_at(50.0)
                && vp.ground_speed_at(50.0) < vp.ground_speed_at(150.0),
            "the descent must still slow monotonically toward the field: 20nm={:.1} 50nm={:.1} \
             150nm={:.1}",
            vp.ground_speed_at(20.0),
            vp.ground_speed_at(50.0),
            vp.ground_speed_at(150.0)
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
        flown(35_000.0, start_d, 0.0, 35_000.0, tas, profile, wind)
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
        // The two step grids differ by a step's altitude, about 1 kt, not by any wind.
        assert!((windy.ground_speed_at(5.0) - still.ground_speed_at(5.0)).abs() < 3.0);
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
            (274.0, 0.0, None),
            (276.0, 100.0, None),
            (400.0, 100.0, None),
        ]);
        let per_segment = cruise(400.0, 440.0, &p, wind).time_between(400.0, 150.0);
        let mean = cruise(400.0, 440.0, &p, RouteWind::uniform(50.0)).time_between(400.0, 150.0);
        // 1/(TAS − hw) is convex: at a flat 440 kt, 125/340 + 125/440 h is 2,346 s against
        // 250/390 h = 2,308 s.
        assert!(per_segment > mean + 30.0, "{per_segment} vs {mean}");
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
        let build = |w| flown(0.0, 300.0, 0.0, 35_000.0, 440.0, &p, w);
        let (windy, still) = (build(wind), build(RouteWind::still()));
        let climb_nm = |vp: &VerticalProfile| {
            let mut d = 300.0;
            while d > 0.0 && vp.alt_at(d) < 34_400.0 {
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
        let hot = cruise(400.0, 520.0, &mach_cruiser(), warm);
        let isa = cruise(400.0, 520.0, &mach_cruiser(), RouteWind::still());
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

    // ISA where the field has no temperature (any replayed FB snapshot; a RAP point with TMP
    // missing).
    #[test]
    fn a_field_without_temperatures_flies_mach_on_isa() {
        let wind = RouteWind::new(vec![knot(0.0, vec![(35_000.0, 40.0, None)])]);
        let vp = cruise(400.0, 520.0, &mach_cruiser(), wind);
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
            let build = |w| flown(0.0, 300.0, 0.0, alt, tas, &p, w);
            let (hot, isa) = (build(warm.clone()), build(RouteWind::still()));
            assert!((hot.time_between(300.0, 0.0) - isa.time_between(300.0, 0.0)).abs() < 1e-9);
        }
    }

    // Each step reads the temperature where it is, so a change mid-cruise changes the Mach TAS there.
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
        let vp = cruise(600.0, 520.0, &mach_cruiser(), wind);
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

    // Into a headwind stronger than the climb and descent speeds, each step is flown at the
    // groundspeed floor (60 kt): a 500 fpm climb through 5,000 ft covers 10 nm, and 10 nm of descent
    // takes 10 minutes.
    #[test]
    fn climb_and_descent_into_a_headwind_past_their_tas_fly_the_groundspeed_floor() {
        let wind = flat(&[(0.0, 200.0, None), (200.0, 200.0, None)]);
        let vp = flown(0.0, 200.0, 0.0, 10_000.0, 110.0, &c172(), wind);
        assert!(
            (vp.alt_at(190.0) - 5_000.0).abs() < 1e-6,
            "{}",
            vp.alt_at(190.0)
        );
        let last_ten = vp.time_between(10.0, 0.0);
        assert!((last_ten - 600.0).abs() < 1e-6, "{last_ten}");
    }

    // Anchoring divides by the cruise speed the profile flies in the air where it is, not the ISA speed nor the
    // arrival field's: a Mach 0.84 cruise at ISA+15 flies 500.5 kt where ISA would give 484.2 kt.
    #[test]
    fn anchoring_a_mach_cruise_off_isa_lands_on_the_observed_groundspeed() {
        let warm = flat(&[
            (0.0, 0.0, Some(ISA_FL350_K + 15.0)),
            (400.0, 0.0, Some(ISA_FL350_K + 15.0)),
        ]);
        let vp = anchored(400.0, 520.0, &mach_cruiser(), warm, 520.0);
        assert!(
            (vp.ground_speed_at(400.0) - 520.0).abs() < 1e-6,
            "{}",
            vp.ground_speed_at(400.0)
        );
        let warm_there = flat(&[
            (0.0, 0.0, Some(ISA_FL350_K)),
            (400.0, 0.0, Some(ISA_FL350_K + 15.0)),
        ]);
        let vp = anchored(400.0, 520.0, &mach_cruiser(), warm_there, 520.0);
        assert!((vp.ground_speed_at(400.0) - 520.0).abs() < 1e-6);
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

    // by_altitude brackets with the nearest level above, not the highest, when three or more
    // levels carry a value.
    #[test]
    fn route_wind_brackets_with_the_nearest_level_above() {
        let w = RouteWind::new(vec![knot(
            0.0,
            vec![
                (10_000.0, 10.0, Some(250.0)),
                (20_000.0, 30.0, Some(240.0)),
                (30_000.0, 90.0, Some(200.0)),
            ],
        )]);
        assert_eq!(w.headwind(0.0, 15_000.0), Some(20.0));
        assert_eq!(w.temp_k(0.0, 15_000.0), Some(245.0));
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

    /// Knots every 50 nm from 0 to 600 nm out, covered (with these levels) only at 450 and 500 nm
    /// and gaps everywhere else: covered values hold from 400 to 550 nm, and the air is still
    /// beyond.
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
        let vp = cruise(
            600.0,
            520.0,
            &mach_cruiser(),
            covered_450_to_500(warm.clone()),
        );
        let isa = cruise(600.0, 520.0, &mach_cruiser(), RouteWind::still());
        let hot = cruise(
            600.0,
            520.0,
            &mach_cruiser(),
            RouteWind::new(vec![knot(0.0, warm)]),
        );
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
        // One step at each end may straddle the gap, 1.8 nm of the 150.
        assert!((vp.mean_headwind(550.0, 400.0).unwrap() - 80.0).abs() < 1.0);
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

    // The span between the last two samples (the departure end) interpolates like any other: a
    // departure from the surface is part way up its first 500 ft climb step just after the start,
    // not held at the start sample's altitude.
    #[test]
    fn the_profile_interpolates_inside_its_last_segment() {
        let p = AircraftProfile::default();
        let vp = flown(0.0, 300.0, 0.0, 35_000.0, 440.0, &p, RouteWind::still());
        // The first climb step: 500 ft at 2,000 fpm is 15 s, about 1 nm at the climb speed.
        let a = vp.alt_at(299.9);
        assert!(a > 10.0 && a < 500.0, "0.1 nm into the climb at {a} ft");
        assert!(
            vp.alt_at(299.95) < a,
            "the altitude rises away from the start"
        );
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
        let build = |w| flown(0.0, 300.0, 0.0, 35_000.0, 520.0, &mach_cruiser(), w);
        let (warm, isa) = (build(warm_below_cruise()), build(RouteWind::still()));
        // The last 150 nm hold the whole descent, and the climb is over before them.
        let saved = isa.time_between(150.0, 0.0) - warm.time_between(150.0, 0.0);
        assert!(saved > 0.5, "the warm Mach descent saves {saved} s");
    }

    // The descent reads the temperature at each step's own distance: the same Mach band is warm 95 nm
    // out and ISA 86 nm out. The two marches' steps fall at different distances, so the speeds compared
    // are up to a step's altitude apart (about 1.5 kt of a constant Mach).
    #[test]
    fn a_descent_reads_the_temperature_where_each_step_is() {
        let isa = |ft: f64| 288.15 - 0.0019812 * ft;
        let band = |dt: f64| {
            vec![
                (25_000.0, 0.0, Some(isa(25_000.0) + dt)),
                (35_000.0, 0.0, Some(ISA_FL350_K + dt)),
            ]
        };
        let far_warm = RouteWind::new(vec![
            knot(0.0, band(0.0)),
            knot(88.0, band(0.0)),
            knot(92.0, band(15.0)),
            knot(300.0, band(15.0)),
        ]);
        let build = |w| flown(0.0, 300.0, 0.0, 35_000.0, 520.0, &mach_cruiser(), w);
        let (warm, isa_p) = (build(far_warm), build(RouteWind::still()));
        let faster = |d: f64| warm.ground_speed_at(d) - isa_p.ground_speed_at(d);
        // 661.4788 x 0.84 x (sqrt(T + 15) - sqrt(T)) / sqrt(288.15) at ~31,700 ft: 15.9 kt.
        assert!((faster(95.0) - 15.9).abs() < 1.5, "{}", faster(95.0));
        assert!(faster(86.0).abs() < 2.0, "{}", faster(86.0));
    }

    // The same on the climb: the Mach part of it is faster in warm air.
    #[test]
    fn a_climb_mach_flies_faster_in_warm_air() {
        let build = |w| flown(0.0, 300.0, 0.0, 35_000.0, 520.0, &mach_cruiser(), w);
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
        let build = |w| flown(0.0, 300.0, 0.0, 35_000.0, 520.0, &mach_cruiser(), w);
        let (warm_start, isa_air) = (build(wind), build(RouteWind::still()));
        let gained = isa_air.time_between(300.0, 150.0) - warm_start.time_between(300.0, 150.0);
        assert!(gained.abs() < 0.05, "the Mach climb is on ISA: {gained} s");
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

// Coverage matrix (VATUSA/OIS#818)
//
// Requirement                                         | Test
// ----------------------------------------------------|----------------------------------------------------------
// AC1 resolve order type -> family -> wake -> default | resolve_falls_through_type_family_wake_default_in_that_order
// AC1 family keyed on the type's own family           | a_family_row_for_another_engine_family_is_not_used
// AC1 a type outside ENGINE_REF skips family          | a_type_outside_the_engine_reference_skips_the_family_tier
// AC1 label agrees with resolve at every tier         | resolve_label_names_the_tier_resolve_used_at_every_tier
// AC1 no family rows behaves as next                  | with_no_family_rows_resolution_is_type_then_wake_then_default
#[cfg(test)]
mod profile_table_family_tests {
    // B772 is a jet in `feed::flow::ENGINE_REF` with no 0059 seed row; AT72 a turboprop; QQQQ is in
    // neither. Each tier carries its own literal `desc_fpm`, so the profile `resolve` returns names
    // the tier it came from.
    use std::collections::HashMap;

    use super::{AircraftProfile, Performance, ProfileTable};

    fn fpm(desc_fpm: f64) -> AircraftProfile {
        AircraftProfile {
            desc_fpm,
            ..AircraftProfile::default()
        }
    }

    /// The table with the tiers named in `tiers` ("type", "family", "wake", "default"); the
    /// default tier is 4_444 when named, else `AircraftProfile::default()`.
    fn table(tiers: &[&str]) -> ProfileTable {
        let has = |t: &str| tiers.contains(&t);
        let map = |on: bool, rows: &[(&str, f64)]| -> HashMap<String, AircraftProfile> {
            if !on {
                return HashMap::new();
            }
            rows.iter().map(|(k, v)| (k.to_string(), fpm(*v))).collect()
        };
        ProfileTable::new(
            if has("default") {
                fpm(4_444.0)
            } else {
                AircraftProfile::default()
            },
            map(has("wake"), &[("H", 3_333.0)]),
            map(has("family"), &[("jet", 2_222.0), ("turboprop", 2_777.0)]),
            map(has("type"), &[("B772", 1_111.0)]),
            None,
        )
    }

    /// The `desc_fpm` of the row a flight resolves to.
    fn fpm_of(t: &ProfileTable, ty: &str, wake: &str) -> f64 {
        match t.resolve(ty, wake) {
            Performance::Row(p) => p.desc_fpm,
            Performance::Eram(_) => panic!("{ty} resolved to ERAM"),
        }
    }

    #[test]
    fn resolve_falls_through_type_family_wake_default_in_that_order() {
        let steps: [(&[&str], f64); 4] = [
            (&["type", "family", "wake", "default"], 1_111.0),
            (&["family", "wake", "default"], 2_222.0),
            (&["wake", "default"], 3_333.0),
            (&["default"], 4_444.0),
        ];
        for (tiers, want) in steps {
            assert_eq!(fpm_of(&table(tiers), "B772", "H"), want, "tiers {tiers:?}");
        }
    }

    #[test]
    fn a_family_row_for_another_engine_family_is_not_used() {
        let t = ProfileTable::new(
            fpm(4_444.0),
            HashMap::new(),
            HashMap::from([("turboprop".to_string(), fpm(2_777.0))]),
            HashMap::new(),
            None,
        );
        assert_eq!(
            fpm_of(&t, "B772", ""),
            4_444.0,
            "a jet ignores the turboprop row"
        );
        assert_eq!(fpm_of(&t, "AT72", ""), 2_777.0, "a turboprop takes it");
    }

    #[test]
    fn a_type_outside_the_engine_reference_skips_the_family_tier() {
        let t = table(&["type", "family", "wake", "default"]);
        assert_eq!(fpm_of(&t, "QQQQ", "H"), 3_333.0);
        assert_eq!(fpm_of(&t, "QQQQ", ""), 4_444.0);
        assert_eq!(fpm_of(&t, "B772", ""), 1_111.0, "control: type row");
        let no_type = table(&["family", "wake", "default"]);
        assert_eq!(fpm_of(&no_type, "B772", ""), 2_222.0, "control: family row");
    }

    #[test]
    fn resolve_label_names_the_tier_resolve_used_at_every_tier() {
        let steps: [(&[&str], &str, f64); 4] = [
            (&["type", "family", "wake", "default"], "type:B772", 1_111.0),
            (&["family", "wake", "default"], "family:jet", 2_222.0),
            (&["wake", "default"], "wake:H", 3_333.0),
            (&["default"], "default", 4_444.0),
        ];
        for (tiers, label, want) in steps {
            let t = table(tiers);
            assert_eq!(t.resolve_label("B772", "H"), label, "tiers {tiers:?}");
            assert_eq!(fpm_of(&t, "B772", "H"), want, "tiers {tiers:?}");
        }
        let lower = table(&["family", "wake", "default"]);
        assert_eq!(lower.resolve_label("b772", "h"), "family:jet");
        assert_eq!(fpm_of(&lower, "b772", "h"), 2_222.0);
        assert_eq!(lower.resolve_label("at72", ""), "family:turboprop");
        let wake_only = table(&["wake", "default"]);
        assert_eq!(wake_only.resolve_label("QQQQ", "h"), "wake:H");
        assert_eq!(fpm_of(&wake_only, "QQQQ", "h"), 3_333.0);
        let piston = ProfileTable::new(
            fpm(4_444.0),
            HashMap::new(),
            HashMap::from([("piston".to_string(), fpm(1_000.0))]),
            HashMap::new(),
            None,
        );
        assert_eq!(piston.resolve_label("PA34", ""), "family:piston");
        assert_eq!(fpm_of(&piston, "PA34", ""), 1_000.0);
    }

    #[test]
    fn with_no_family_rows_resolution_is_type_then_wake_then_default() {
        let t = table(&["type", "wake", "default"]);
        assert_eq!(fpm_of(&t, "B772", "H"), 1_111.0);
        assert_eq!(fpm_of(&t, "B738", "H"), 3_333.0);
        assert_eq!(fpm_of(&t, "B738", ""), 4_444.0);
        assert_eq!(t.resolve_label("B772", "H"), "type:B772");
        assert_eq!(t.resolve_label("B738", "H"), "wake:H");
        assert_eq!(t.resolve_label("B738", ""), "default");
    }
}

#[path = "trajectory_march_tests.rs"]
#[cfg(test)]
mod trajectory_march_tests;
