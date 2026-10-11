//! Aggregating per-flight values into per-type profiles, the reviewable output file, and the
//! coverage report (#828).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::samples::Field;
use crate::feed::trajectory::AircraftProfile;

/// A field is measured for a type only when at least this many training flights contributed a value
/// to it. Twenty keeps one odd flight (a sim on autopilot V/S 4000, a pilot holding 250 kt to FL200)
/// from moving the median, while still reaching the regional and business types the 0059 seed never
/// covered. A type below it on any phase keeps that phase's current profile, and the coverage report
/// lists it with its wake class and engine family so #818 can give it a family fallback.
pub const MIN_FLIGHTS_PER_FIELD: usize = 20;

/// The median of `v` (unsorted), or None when empty.
pub fn median(v: &[f64]) -> Option<f64> {
    quantile(v, 0.5)
}

/// The `q`-quantile of `v` (unsorted) with linear interpolation between closest ranks, or None when
/// empty.
pub fn quantile(v: &[f64], q: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let pos = q.clamp(0.0, 1.0) * (s.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    Some(s[lo] + (pos - lo as f64) * (s[hi] - s[lo]))
}

/// The spread of one field across a type's flights. Speeds and rates are rounded to whole knots and
/// feet per minute, Mach numbers to three decimals, so the file is stable to review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldStat {
    pub median: f64,
    pub q1: f64,
    pub q3: f64,
    /// Training flights that contributed a value.
    pub flights: usize,
}

impl FieldStat {
    fn of(field: Field, per_flight: &[f64]) -> Option<Self> {
        let r = |x: f64| {
            if field.is_mach() {
                (x * 1000.0).round() / 1000.0
            } else {
                x.round()
            }
        };
        Some(Self {
            median: r(median(per_flight)?),
            q1: r(quantile(per_flight, 0.25)?),
            q3: r(quantile(per_flight, 0.75)?),
            flights: per_flight.len(),
        })
    }

    pub fn measured(&self) -> bool {
        self.flights >= MIN_FLIGHTS_PER_FIELD
    }
}

/// Where the numbers came from. Holds no run timestamp, so the same data gives the same file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// The `--from` / `--to` window (RFC 3339): flights wholly inside it were read.
    pub from: String,
    pub to: String,
    pub flights_read: usize,
    pub training_flights: usize,
    pub held_out_flights: usize,
    pub held_out_rule: String,
}

/// One aircraft type's measured performance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeProfile {
    /// Training flights of this type.
    pub flights: usize,
    /// The wake class most of its flight plans carry (`H`, `J`; see `profile_builder::wake_of`), or
    /// None when they carry none.
    pub wake: Option<String>,
    /// `jet`, `turboprop` or `piston` from `feed::flow`'s engine reference, or None when unlisted.
    pub engine: Option<String>,
    /// True when climb, cruise and descent each have at least one measured field.
    pub profiled: bool,
    /// Keyed by the `AircraftProfile` field name.
    pub fields: BTreeMap<String, FieldStat>,
}

/// The builder's output file (`backend/data/aircraft_profiles.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSet {
    pub source: Source,
    pub min_flights_per_field: usize,
    pub types: BTreeMap<String, TypeProfile>,
}

/// Collects each flight's per-field values by type.
#[derive(Default)]
pub struct Accumulator {
    by_type: BTreeMap<String, TypeAcc>,
}

#[derive(Default)]
struct TypeAcc {
    flights: usize,
    wakes: BTreeMap<String, usize>,
    values: BTreeMap<Field, Vec<f64>>,
}

impl Accumulator {
    pub fn add_flight(&mut self, ty: &str, wake: &str, per_flight: &[(Field, f64)]) {
        let acc = self.by_type.entry(ty.to_string()).or_default();
        acc.flights += 1;
        *acc.wakes.entry(wake.to_string()).or_default() += 1;
        for (f, v) in per_flight {
            acc.values.entry(*f).or_default().push(*v);
        }
    }

    pub fn finish(
        self,
        source: Source,
        engine_of: impl Fn(&str) -> Option<&'static str>,
    ) -> ProfileSet {
        let types = self
            .by_type
            .into_iter()
            .map(|(ty, acc)| {
                let fields: BTreeMap<String, FieldStat> = acc
                    .values
                    .iter()
                    .filter_map(|(f, v)| FieldStat::of(*f, v).map(|s| (f.name().to_string(), s)))
                    .collect();
                let has = |fs: &[Field]| {
                    fs.iter()
                        .any(|f| fields.get(f.name()).is_some_and(FieldStat::measured))
                };
                let profiled = has(&[Field::ClimbFpmLo, Field::ClimbFpmHi])
                    && has(&[Field::CruiseTas])
                    && has(&[Field::DescFpm]);
                // The most common wake class; ties go to the alphabetically first, so it is stable.
                let wake = acc
                    .wakes
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                    .map(|(w, _)| w.clone())
                    .filter(|w| !w.is_empty());
                let tp = TypeProfile {
                    flights: acc.flights,
                    wake,
                    engine: engine_of(&ty).map(str::to_string),
                    profiled,
                    fields,
                };
                (ty, tp)
            })
            .collect();
        ProfileSet {
            source,
            min_flights_per_field: MIN_FLIGHTS_PER_FIELD,
            types,
        }
    }
}

impl TypeProfile {
    /// `base` with every measured field replaced by the measurement. Schedule values (IAS, climb and
    /// descent Mach, rates) take the median. `cruise_tas` and `cruise_mach` are **caps** in the
    /// model (`trajectory::capped_cruise_tas`, `VerticalProfile::build`), so they take the upper
    /// quartile: a median cap would slow down half the flights that really fly faster.
    pub fn apply_to(&self, base: &AircraftProfile) -> AircraftProfile {
        let mut p = base.clone();
        for (name, s) in &self.fields {
            if !s.measured() {
                continue;
            }
            let Some(f) = Field::from_name(name) else {
                continue;
            };
            match f {
                Field::ClimbIasLo => p.climb_ias_lo = s.median,
                Field::ClimbIasHi => p.climb_ias_hi = s.median,
                Field::ClimbMach => p.climb_mach = Some(s.median),
                Field::ClimbFpmLo => p.climb_fpm_lo = s.median,
                Field::ClimbFpmHi => p.climb_fpm_hi = s.median,
                Field::CruiseTas => p.cruise_tas = Some(s.q3),
                Field::CruiseMach => p.cruise_mach = Some(s.q3),
                Field::DescMach => p.desc_mach = Some(s.median),
                Field::DescIasHi => p.desc_ias_hi = s.median,
                Field::DescIasLo => p.desc_ias_lo = s.median,
                Field::DescFpm => p.desc_fpm = s.median,
            }
        }
        p
    }
}

impl ProfileSet {
    /// The coverage report as Markdown: per-type sample counts and the share of traffic covered.
    /// Aggregates only; nothing in it identifies a pilot or a flight.
    pub fn coverage_report(&self) -> String {
        let mut types: Vec<(&String, &TypeProfile)> = self.types.iter().collect();
        types.sort_by(|a, b| b.1.flights.cmp(&a.1.flights).then(a.0.cmp(b.0)));
        let total: usize = types.iter().map(|t| t.1.flights).sum();
        let (prof, thin): (Vec<_>, Vec<_>) = types.into_iter().partition(|t| t.1.profiled);
        let covered: usize = prof.iter().map(|t| t.1.flights).sum();
        let pct = |n: usize| {
            if total == 0 {
                0.0
            } else {
                n as f64 * 100.0 / total as f64
            }
        };
        let s = &self.source;
        let mut out = String::new();
        out.push_str("# Aircraft profile coverage\n\n");
        out.push_str(&format!(
            "Window {} to {}. {} flights read: {} training, {} held out ({}).\n\n",
            s.from, s.to, s.flights_read, s.training_flights, s.held_out_flights, s.held_out_rule
        ));
        out.push_str(&format!(
            "A field needs {} training flights. {} of {} types are profiled (climb, cruise and \
             descent each measured), covering {} of {} training flights ({:.1}%).\n\n",
            self.min_flights_per_field,
            prof.len(),
            prof.len() + thin.len(),
            covered,
            total,
            pct(covered)
        ));
        let cell = |t: &TypeProfile, f: Field| match t.fields.get(f.name()) {
            Some(s) if s.measured() => format!("{} ({})", fmt_num(s.median, f), s.flights),
            Some(s) => format!("- ({})", s.flights),
            None => "-".to_string(),
        };
        out.push_str("## Profiled types\n\nMedian (flights) per field.\n\n");
        out.push_str(
            "| Type | Flights | Climb fpm <10k | Climb fpm >=10k | Climb KIAS >=10k | Cruise KTAS | \
             Cruise Mach | Descent fpm | Descent KIAS >=10k |\n|---|---|---|---|---|---|---|---|---|\n",
        );
        for (ty, t) in &prof {
            out.push_str(&format!(
                "| {ty} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                t.flights,
                cell(t, Field::ClimbFpmLo),
                cell(t, Field::ClimbFpmHi),
                cell(t, Field::ClimbIasHi),
                cell(t, Field::CruiseTas),
                cell(t, Field::CruiseMach),
                cell(t, Field::DescFpm),
                cell(t, Field::DescIasHi),
            ));
        }
        out.push_str(&format!(
            "\n## Below the threshold\n\n{} types, {} training flights ({:.1}%).\n\n\
             | Type | Flights | Wake | Engine |\n|---|---|---|---|\n",
            thin.len(),
            total - covered,
            pct(total - covered)
        ));
        for (ty, t) in &thin {
            out.push_str(&format!(
                "| {ty} | {} | {} | {} |\n",
                t.flights,
                t.wake.as_deref().unwrap_or("-"),
                t.engine.as_deref().unwrap_or("unknown"),
            ));
        }
        out
    }
}

fn fmt_num(v: f64, f: Field) -> String {
    if f.is_mach() {
        format!("{v:.3}")
    } else {
        format!("{v:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_interpolate_between_ranks() {
        let v = [40.0, 10.0, 30.0, 20.0];
        assert_eq!(median(&v), Some(25.0));
        assert_eq!(quantile(&v, 0.25), Some(17.5));
        assert_eq!(quantile(&v, 0.75), Some(32.5));
        assert_eq!(median(&[7.0]), Some(7.0));
        assert_eq!(median(&[]), None);
    }

    fn source() -> Source {
        Source {
            from: "2026-09-20T00:00:00+00:00".into(),
            to: "2026-10-10T00:00:00+00:00".into(),
            flights_read: 0,
            training_flights: 0,
            held_out_flights: 0,
            held_out_rule: "test".into(),
        }
    }

    fn full_flight(fpm: f64) -> Vec<(Field, f64)> {
        vec![
            (Field::ClimbFpmHi, fpm),
            (Field::CruiseTas, 440.0 + fpm / 100.0),
            (Field::DescFpm, 2_000.0),
        ]
    }

    #[test]
    fn a_type_is_profiled_only_at_the_threshold() {
        let mut acc = Accumulator::default();
        for k in 0..MIN_FLIGHTS_PER_FIELD {
            acc.add_flight("B738", "", &full_flight(2_000.0 + k as f64));
        }
        for k in 0..MIN_FLIGHTS_PER_FIELD - 1 {
            acc.add_flight("CRJ2", "", &full_flight(2_500.0 + k as f64));
        }
        let set = acc.finish(source(), |t| (t == "CRJ2").then_some("jet"));
        assert!(set.types["B738"].profiled);
        assert!(
            !set.types["CRJ2"].profiled,
            "one flight short of the threshold"
        );
        assert_eq!(set.types["CRJ2"].engine.as_deref(), Some("jet"));
        let report = set.coverage_report();
        assert!(report.contains("| CRJ2 | 19 | - | jet |"), "{report}");
        assert!(report.contains("1 of 2 types are profiled"), "{report}");
    }

    #[test]
    fn caps_take_the_upper_quartile_and_schedules_the_median() {
        let mut acc = Accumulator::default();
        for k in 0..20 {
            acc.add_flight(
                "A320",
                "",
                &[
                    (Field::CruiseTas, 400.0 + k as f64),
                    (Field::DescFpm, 1_000.0 + 100.0 * k as f64),
                ],
            );
        }
        let set = acc.finish(source(), |_| None);
        let p = set.types["A320"].apply_to(&AircraftProfile::default());
        // 400..419: q3 = 414.25 → 414; desc 1000..2900: median 1950.
        assert_eq!(p.cruise_tas, Some(414.0));
        assert_eq!(p.desc_fpm, 1_950.0);
        // Unmeasured fields keep the base profile's values.
        assert_eq!(p.climb_fpm_lo, AircraftProfile::default().climb_fpm_lo);
    }

    #[test]
    fn a_field_below_the_threshold_leaves_the_base_value() {
        let mut acc = Accumulator::default();
        for _ in 0..MIN_FLIGHTS_PER_FIELD - 1 {
            acc.add_flight("C172", "", &[(Field::DescFpm, 500.0)]);
        }
        let set = acc.finish(source(), |_| None);
        assert_eq!(
            set.types["C172"].apply_to(&AircraftProfile::default()),
            AircraftProfile::default()
        );
    }

    #[test]
    fn the_majority_wake_wins_and_blank_means_none() {
        let mut acc = Accumulator::default();
        acc.add_flight("B77W", "H", &[]);
        acc.add_flight("B77W", "H", &[]);
        acc.add_flight("B77W", "", &[]);
        acc.add_flight("B738", "", &[]);
        let set = acc.finish(source(), |_| None);
        assert_eq!(set.types["B77W"].wake.as_deref(), Some("H"));
        assert_eq!(set.types["B738"].wake, None);
    }
}
