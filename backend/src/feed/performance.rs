//! Aircraft performance: what a type flies at an altitude (#985).
//!
//! Two sources sit behind one [`Performance`]. [`EramRecord`]s are ERAM's own per-type tables (climb
//! and descent rate and true airspeed by altitude, and the true-airspeed range a filed cruise speed
//! is held inside), bundled from `backend/data/aircraft-performance.json`. An [`AircraftProfile`] is a
//! row of `flow.aircraft_profile`: an indicated-airspeed and Mach schedule. Which one a flight gets is
//! [`super::trajectory::ProfileTable::resolve`]'s decision.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use super::trajectory::{
    AircraftProfile, believable_tas, capped_cruise_tas, isa_temp_k, mach_to_tas_at, phase_tas,
};

/// `[altitude ft, rate fpm, TAS kt]`, in increasing altitude.
type RateRow = [f64; 3];
/// `[altitude ft, TAS kt]`, in increasing altitude.
type SpeedRow = [f64; 2];

/// Vertical step (ft) the still-air climb and descent distances are summed in.
const DISTANCE_STEP_FT: f64 = 500.0;

/// One type's ERAM tables. Values interpolate linearly between rows and hold the end rows beyond them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EramRecord {
    climb: Vec<RateRow>,
    descent: Vec<RateRow>,
    min_tas: Vec<SpeedRow>,
    max_tas: Vec<SpeedRow>,
}

#[cfg(test)]
impl EramRecord {
    pub(crate) fn for_test(
        climb: Vec<RateRow>,
        descent: Vec<RateRow>,
        min_tas: Vec<SpeedRow>,
        max_tas: Vec<SpeedRow>,
    ) -> Self {
        Self {
            climb,
            descent,
            min_tas,
            max_tas,
        }
    }
}

#[cfg(test)]
impl EramTable {
    pub(crate) fn type_count(&self) -> usize {
        self.0.len()
    }
}

#[derive(Deserialize)]
struct EramFile {
    types: HashMap<String, EramRecord>,
}

/// The ERAM records by uppercase type designator.
#[derive(Debug)]
pub struct EramTable(HashMap<String, EramRecord>);

static BUNDLED: LazyLock<EramTable> = LazyLock::new(|| {
    EramTable::parse(include_str!("../../data/aircraft-performance.json"))
        .expect("the bundled ERAM file is valid: performance_tests parses it")
});

impl EramTable {
    /// The table compiled into the binary.
    pub fn bundled() -> &'static Self {
        &BUNDLED
    }

    /// Parse the bundled file's format, refusing a type that lacks any of its four tables.
    fn parse(json: &str) -> Result<Self, String> {
        let file: EramFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if file.types.is_empty() {
            return Err("no types".into());
        }
        for (ty, r) in &file.types {
            if r.climb.is_empty()
                || r.descent.is_empty()
                || r.min_tas.is_empty()
                || r.max_tas.is_empty()
            {
                return Err(format!("{ty} lacks a table"));
            }
        }
        Ok(Self(file.types))
    }

    /// The record of an uppercase type designator.
    pub fn get(&self, ty: &str) -> Option<&EramRecord> {
        self.0.get(ty)
    }
}

/// Column `col` of `rows` (never empty, altitude ascending) at `alt_ft`.
fn interp<const N: usize>(rows: &[[f64; N]], alt_ft: f64, col: usize) -> f64 {
    let i = rows.partition_point(|r| r[0] < alt_ft);
    if i == 0 {
        return rows[0][col];
    }
    let Some(hi) = rows.get(i) else {
        return rows[i - 1][col];
    };
    let lo = &rows[i - 1];
    lo[col] + (alt_ft - lo[0]) / (hi[0] - lo[0]) * (hi[col] - lo[col])
}

/// The factor ERAM's standard-day climb and descent speeds are multiplied by in air at `temp_k`: the
/// square root of its ratio to the standard temperature, 1 where the winds carry none.
fn temperature_factor(alt_ft: f64, temp_k: Option<f64>) -> f64 {
    temp_k.map_or(1.0, |t| (t.max(1.0) / isa_temp_k(alt_ft.max(0.0))).sqrt())
}

/// What a flight's type flies.
#[derive(Debug, Clone, Copy)]
pub enum Performance<'a> {
    Eram(&'a EramRecord),
    Row(&'a AircraftProfile),
}

impl Performance<'_> {
    /// Climb true airspeed (kt) and rate (fpm) at `alt_ft` in air at `temp_k` (None: standard).
    pub fn climb(&self, alt_ft: f64, temp_k: Option<f64>) -> (f64, f64) {
        match self {
            Self::Eram(r) => (
                interp(&r.climb, alt_ft, 2) * temperature_factor(alt_ft, temp_k),
                interp(&r.climb, alt_ft, 1),
            ),
            Self::Row(p) => {
                let temp = temp_k.unwrap_or_else(|| isa_temp_k(alt_ft.max(0.0)));
                let fpm = if alt_ft < 10_000.0 {
                    p.climb_fpm_lo
                } else {
                    p.climb_fpm_hi
                };
                (
                    phase_tas(p.climb_ias_lo, p.climb_ias_hi, p.climb_mach, alt_ft, temp),
                    fpm.max(1.0),
                )
            }
        }
    }

    /// Descent true airspeed (kt) at `alt_ft` in air at `temp_k`, never above the cruise there (#361).
    pub fn descent_tas(&self, alt_ft: f64, temp_k: Option<f64>, cruise_tas: f64) -> f64 {
        self.own_descent_tas(alt_ft, temp_k)
            .min(self.cruise_in(cruise_tas, alt_ft, temp_k))
    }

    /// The descent schedule or table as it stands, before the cruise cap.
    fn own_descent_tas(&self, alt_ft: f64, temp_k: Option<f64>) -> f64 {
        match self {
            Self::Eram(r) => interp(&r.descent, alt_ft, 2) * temperature_factor(alt_ft, temp_k),
            Self::Row(p) => {
                let temp = temp_k.unwrap_or_else(|| isa_temp_k(alt_ft.max(0.0)));
                phase_tas(p.desc_ias_lo, p.desc_ias_hi, p.desc_mach, alt_ft, temp)
            }
        }
    }

    /// Cruise true airspeed (kt) in air at `temp_k`: a row with a cruise Mach is held to it.
    pub fn cruise_in(&self, cruise_tas: f64, alt_ft: f64, temp_k: Option<f64>) -> f64 {
        match self {
            Self::Row(AircraftProfile {
                cruise_mach: Some(m),
                ..
            }) => {
                let temp = temp_k.unwrap_or_else(|| isa_temp_k(alt_ft.max(0.0)));
                cruise_tas.min(mach_to_tas_at(*m, temp))
            }
            _ => cruise_tas,
        }
    }

    /// The cruise true airspeed (kt) a flight filed at `filed_tas` and `cruise_ft` flies at
    /// `level_ft`. An ERAM type holds a believable filing inside its range there and cruises at the
    /// range's midpoint otherwise; a row caps the filing ([`capped_cruise_tas`]).
    pub fn cruise_tas(&self, filed_tas: f64, cruise_ft: f64, level_ft: f64) -> f64 {
        match self {
            Self::Eram(r) => {
                let min = interp(&r.min_tas, level_ft, 1);
                let max = interp(&r.max_tas, level_ft, 1);
                if believable_tas(filed_tas) {
                    filed_tas.max(min).min(max)
                } else {
                    (min + max) / 2.0
                }
            }
            Self::Row(p) => capped_cruise_tas(filed_tas, cruise_ft, p),
        }
    }

    /// The altitude (ft) the type cannot cruise above: a row's service ceiling. The ERAM file's maximum
    /// altitudes are not performance limits (a C172 reads 45,000 ft), so it gives none.
    pub fn ceiling_ft(&self) -> Option<f64> {
        match self {
            Self::Eram(_) => None,
            Self::Row(p) => Some(p.service_ceiling_ft),
        }
    }

    /// Still-air, standard-day distance (nm) the type covers descending from `from_ft` to `to_ft`. A row
    /// descends no faster than `cruise_tas`.
    pub fn descent_distance_nm(&self, from_ft: f64, to_ft: f64, cruise_tas: f64) -> f64 {
        distance_nm(to_ft, from_ft, |alt| match self {
            Self::Eram(r) => (interp(&r.descent, alt, 2), interp(&r.descent, alt, 1)),
            Self::Row(p) => (self.descent_tas(alt, None, cruise_tas), p.desc_fpm),
        })
    }

    /// Still-air, standard-day distance (nm) the type covers climbing from the surface to `to_ft`.
    pub fn climb_distance_nm(&self, to_ft: f64) -> f64 {
        distance_nm(0.0, to_ft, |alt| self.climb(alt, None))
    }
}

/// Distance (nm) flown between two altitudes at the `(TAS kt, rate fpm)` of each step's middle.
fn distance_nm(lo_ft: f64, hi_ft: f64, at: impl Fn(f64) -> (f64, f64)) -> f64 {
    let mut nm = 0.0;
    let mut alt = hi_ft;
    while alt > lo_ft {
        let step = DISTANCE_STEP_FT.min(alt - lo_ft);
        let (tas, fpm) = at(alt - step / 2.0);
        nm += tas * (step / fpm.max(1.0)) / 60.0;
        alt -= step;
    }
    nm
}

// `#[path]` before `#[cfg(test)]`: `predict`'s entry-point ratchet finds a test-only file by a
// `#[cfg(test)]` line directly above its `mod name;`.
#[path = "performance_tests.rs"]
#[cfg(test)]
mod performance_tests;
