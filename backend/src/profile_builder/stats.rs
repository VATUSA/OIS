//! Aggregating per-flight values into per-type profiles, the reviewable output file, and the
//! coverage report (#828).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::samples::Field;
use crate::feed::trajectory::AircraftProfile;

/// A field is measured for a type only when at least this many training flights contributed a value
/// to it. Twenty keeps one odd flight (a sim on autopilot V/S 4000, a pilot holding 250 kt to FL200)
/// from moving the median, while still reaching the regional and business types the 0059 seed never
/// covered. A type short of it on climb, cruise or descent is not profiled and keeps its whole
/// current profile; the coverage report lists it with its wake class and engine family so #818 can
/// give it a family fallback.
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
    /// The wake class most of its flight plans carry (`H`, `J`; see
    /// `profile_builder::type_and_wake`), or None when they carry none.
    pub wake: Option<String>,
    /// `jet`, `turboprop` or `piston` from `feed::flow`'s engine reference, or None when unlisted.
    pub engine: Option<String>,
    /// True when climb (`climb_fpm_lo` or `climb_fpm_hi`), cruise (`cruise_tas`) and descent
    /// (`desc_fpm`) each have a measured field.
    pub profiled: bool,
    /// Keyed by the `AircraftProfile` field name.
    pub fields: BTreeMap<String, FieldStat>,
}

/// Every flight of one engine family or one wake class, pooled across types (#818), measured under
/// the same per-field threshold and `profiled` rule as a [`TypeProfile`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PooledProfile {
    /// Training flights in the pool.
    pub flights: usize,
    /// As [`TypeProfile::profiled`].
    pub profiled: bool,
    /// Keyed by the `AircraftProfile` field name.
    pub fields: BTreeMap<String, FieldStat>,
}

/// The builder's output file, written where `--out` says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSet {
    pub source: Source,
    pub min_flights_per_field: usize,
    pub types: BTreeMap<String, TypeProfile>,
    /// Keyed `jet` / `turboprop` / `piston`: every flight of the types `feed::flow`'s engine
    /// reference puts in that family. Absent from a file written before #818.
    #[serde(default)]
    pub families: BTreeMap<String, PooledProfile>,
    /// Keyed by each flight's own wake class, uppercased; a flight with none pools nowhere.
    /// Absent from a file written before #818.
    #[serde(default)]
    pub wakes: BTreeMap<String, PooledProfile>,
}

/// Collects each flight's per-field values by type, and by the wake class the flight carries.
#[derive(Default)]
pub struct Accumulator {
    by_type: BTreeMap<String, TypeAcc>,
    by_wake: BTreeMap<String, PoolAcc>,
}

#[derive(Default)]
struct TypeAcc {
    wakes: BTreeMap<String, usize>,
    pool: PoolAcc,
}

#[derive(Default)]
struct PoolAcc {
    flights: usize,
    values: BTreeMap<Field, Vec<f64>>,
}

impl PoolAcc {
    fn add(&mut self, per_flight: &[(Field, f64)]) {
        self.flights += 1;
        for (f, v) in per_flight {
            self.values.entry(*f).or_default().push(*v);
        }
    }

    fn extend(&mut self, other: &PoolAcc) {
        self.flights += other.flights;
        for (f, v) in &other.values {
            self.values.entry(*f).or_default().extend(v);
        }
    }

    /// The per-field stats, and whether climb, cruise and descent each have a measured field.
    fn measure(&self) -> (BTreeMap<String, FieldStat>, bool) {
        let fields: BTreeMap<String, FieldStat> = self
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
        (fields, profiled)
    }

    fn pooled(&self) -> PooledProfile {
        let (fields, profiled) = self.measure();
        PooledProfile {
            flights: self.flights,
            profiled,
            fields,
        }
    }
}

impl Accumulator {
    pub fn add_flight(&mut self, ty: &str, wake: &str, per_flight: &[(Field, f64)]) {
        let acc = self.by_type.entry(ty.to_string()).or_default();
        acc.pool.add(per_flight);
        *acc.wakes.entry(wake.to_string()).or_default() += 1;
        let wake = wake.trim().to_ascii_uppercase();
        if !wake.is_empty() {
            self.by_wake.entry(wake).or_default().add(per_flight);
        }
    }

    pub fn finish(
        self,
        source: Source,
        engine_of: impl Fn(&str) -> Option<&'static str>,
    ) -> ProfileSet {
        let mut by_family: BTreeMap<String, PoolAcc> = BTreeMap::new();
        for (ty, acc) in &self.by_type {
            if let Some(family) = engine_of(ty) {
                by_family
                    .entry(family.to_string())
                    .or_default()
                    .extend(&acc.pool);
            }
        }
        let types = self
            .by_type
            .into_iter()
            .map(|(ty, acc)| {
                let (fields, profiled) = acc.pool.measure();
                // The most common wake class; ties go to the alphabetically first, so it is stable.
                let wake = acc
                    .wakes
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                    .map(|(w, _)| w.clone())
                    .filter(|w| !w.is_empty());
                let tp = TypeProfile {
                    flights: acc.pool.flights,
                    wake,
                    engine: engine_of(&ty).map(str::to_string),
                    profiled,
                    fields,
                };
                (ty, tp)
            })
            .collect();
        let pool =
            |m: BTreeMap<String, PoolAcc>| m.iter().map(|(k, a)| (k.clone(), a.pooled())).collect();
        ProfileSet {
            source,
            min_flights_per_field: MIN_FLIGHTS_PER_FIELD,
            types,
            families: pool(by_family),
            wakes: pool(self.by_wake),
        }
    }
}

impl TypeProfile {
    /// `base` with every measured field replaced by the measurement. Schedule values (IAS, climb and
    /// descent Mach, rates) take the median. `cruise_tas` and `cruise_mach` are **caps** in the
    /// model (`trajectory::capped_cruise_tas`, `VerticalProfile::build`), so they take the upper
    /// quartile: a median cap would slow down half the flights that really fly faster.
    pub fn apply_to(&self, base: &AircraftProfile) -> AircraftProfile {
        apply_fields(&self.fields, base)
    }
}

impl PooledProfile {
    /// As [`TypeProfile::apply_to`].
    pub fn apply_to(&self, base: &AircraftProfile) -> AircraftProfile {
        apply_fields(&self.fields, base)
    }
}

/// [`TypeProfile::apply_to`] for any measured field map.
fn apply_fields(fields: &BTreeMap<String, FieldStat>, base: &AircraftProfile) -> AircraftProfile {
    let mut p = base.clone();
    for (name, s) in fields {
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

impl ProfileSet {
    /// Refuse a file this build would read differently from how it was written: one measured under
    /// another flight threshold (`measured` uses the compiled one), or one naming a field this build
    /// does not know (`apply_to` would leave it out of set B).
    pub fn check(&self) -> Result<(), String> {
        if self.min_flights_per_field != MIN_FLIGHTS_PER_FIELD {
            return Err(format!(
                "the file was built with min_flights_per_field {}, this build uses {}",
                self.min_flights_per_field, MIN_FLIGHTS_PER_FIELD
            ));
        }
        let types = self.types.iter().map(|(ty, tp)| (ty.clone(), &tp.fields));
        let families = (self.families.iter()).map(|(k, p)| (format!("family:{k}"), &p.fields));
        let wakes = (self.wakes.iter()).map(|(k, p)| (format!("wake:{k}"), &p.fields));
        for (row, fields) in types.chain(families).chain(wakes) {
            if let Some(name) = fields.keys().find(|n| Field::from_name(n).is_none()) {
                return Err(format!("{row}: unknown field {name:?}"));
            }
        }
        Ok(())
    }

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

    /// One measured stat per field, each a distinct value, so every arm of `apply_to` is seen to
    /// write its own profile field: schedules the median, caps the upper quartile.
    #[test]
    fn apply_to_writes_each_field_to_its_own_profile_field() {
        let stat = |k: f64| FieldStat {
            median: k,
            q1: k - 0.5,
            q3: k + 0.5,
            flights: MIN_FLIGHTS_PER_FIELD,
        };
        let tp = TypeProfile {
            flights: MIN_FLIGHTS_PER_FIELD,
            wake: None,
            engine: None,
            profiled: true,
            fields: Field::ALL
                .iter()
                .enumerate()
                .map(|(i, f)| (f.name().to_string(), stat(i as f64 + 1.0)))
                .collect(),
        };
        let p = tp.apply_to(&AircraftProfile::default());
        let got = |f: Field| match f {
            Field::ClimbIasLo => p.climb_ias_lo,
            Field::ClimbIasHi => p.climb_ias_hi,
            Field::ClimbMach => p.climb_mach.unwrap(),
            Field::ClimbFpmLo => p.climb_fpm_lo,
            Field::ClimbFpmHi => p.climb_fpm_hi,
            Field::CruiseTas => p.cruise_tas.unwrap(),
            Field::CruiseMach => p.cruise_mach.unwrap(),
            Field::DescMach => p.desc_mach.unwrap(),
            Field::DescIasHi => p.desc_ias_hi,
            Field::DescIasLo => p.desc_ias_lo,
            Field::DescFpm => p.desc_fpm,
        };
        for (i, f) in Field::ALL.into_iter().enumerate() {
            let cap = matches!(f, Field::CruiseTas | Field::CruiseMach);
            let want = i as f64 + 1.0 + if cap { 0.5 } else { 0.0 };
            assert_eq!(got(f), want, "{}", f.name());
        }
    }

    /// A file from a build with another threshold, or naming a field this build does not know, is
    /// refused; one this build wrote passes.
    #[test]
    fn a_profile_file_this_build_would_misread_is_refused() {
        let mut acc = Accumulator::default();
        acc.add_flight("B738", "", &full_flight(2_000.0));
        let set = acc.finish(source(), |_| None);
        assert_eq!(set.check(), Ok(()));

        let mut other = set.clone();
        other.min_flights_per_field = 10;
        assert_eq!(
            other.check(),
            Err("the file was built with min_flights_per_field 10, this build uses 20".into())
        );
        other.min_flights_per_field = 30;
        assert_eq!(
            other.check(),
            Err("the file was built with min_flights_per_field 30, this build uses 20".into())
        );

        let mut typo = set.clone();
        let stat = typo.types["B738"].fields["desc_fpm"].clone();
        typo.types
            .get_mut("B738")
            .unwrap()
            .fields
            .insert("desc_fmp".into(), stat);
        assert_eq!(typo.check(), Err("B738: unknown field \"desc_fmp\"".into()));
    }

    /// A type is profiled only with climb, cruise and descent each measured: dropping any one of the
    /// three from an otherwise full set leaves it unprofiled.
    #[test]
    fn each_phase_is_needed_to_profile_a_type() {
        let mut acc = Accumulator::default();
        for _ in 0..MIN_FLIGHTS_PER_FIELD {
            acc.add_flight("B738", "", &full_flight(2_000.0));
        }
        assert!(acc.finish(source(), |_| None).types["B738"].profiled);
        for missing in [Field::ClimbFpmHi, Field::CruiseTas, Field::DescFpm] {
            let mut acc = Accumulator::default();
            for _ in 0..MIN_FLIGHTS_PER_FIELD {
                let f: Vec<_> = full_flight(2_000.0)
                    .into_iter()
                    .filter(|(f, _)| *f != missing)
                    .collect();
                acc.add_flight("B738", "", &f);
            }
            let set = acc.finish(source(), |_| None);
            assert!(!set.types["B738"].profiled, "{}", missing.name());
        }
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

// Coverage matrix (VATUSA/OIS#818)
//
// Requirement                                        | Test
// ---------------------------------------------------|-----------------------------------------------------------------------
// AC3 family pooled at exactly 20 (types below 20)   | a_family_is_profiled_at_twenty_pooled_flights_when_no_type_is
// AC3 family one short of 20 is not profiled         | a_family_one_flight_short_of_the_threshold_is_not_profiled
// AC3 profiled types still pool into their family    | a_profiled_type_still_pools_its_flights_into_its_family
// AC3 type with no engine family pools nowhere       | a_type_with_no_engine_family_pools_into_no_family
// AC3 wake keyed on the per-flight wake; blank none  | wake_pools_key_on_each_flights_own_wake_and_a_blank_wake_pools_nowhere
// AC3 wake at / below the 20-flight threshold        | a_wake_pool_is_profiled_only_at_the_threshold
// AC3 two families land in their own slots           | two_families_with_different_values_each_land_in_their_own_row
// AC3 pooled apply_to = TypeProfile::apply_to        | a_pooled_row_applies_to_a_base_exactly_as_a_type_row_does
// AC3 rows written to the output file (round trip)   | families_and_wakes_round_trip_through_the_output_json
// AC3 a pre-#818 file still loads and checks         | a_file_written_before_family_pooling_deserializes_and_passes_check
// AC3 check refuses unknown field in a pooled row    | check_refuses_an_unknown_field_in_a_family_or_wake_row
#[cfg(test)]
mod family_pool_tests {
    //! Counts are literals (20, 19, 10, 9, 11), never derived from `MIN_FLIGHTS_PER_FIELD`; one
    //! assert pins it at 20. `engine_of` is written out here, never the real
    //! `feed::flow::engine_family`, so every type's family is visible in the test.
    use super::super::samples::Field;
    use super::*;

    fn engine_of(t: &str) -> Option<&'static str> {
        match t {
            "CRJ2" | "E75L" | "B738" => Some("jet"),
            "AT72" | "DH8D" => Some("turboprop"),
            "C172" => Some("piston"),
            _ => None,
        }
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

    /// Climb, cruise and descent, so a pool at the threshold is profiled.
    fn flight(fpm: f64, desc_fpm: f64) -> Vec<(Field, f64)> {
        vec![
            (Field::ClimbFpmHi, fpm),
            (Field::CruiseTas, 440.0 + fpm / 100.0),
            (Field::DescFpm, desc_fpm),
        ]
    }

    fn add(acc: &mut Accumulator, ty: &str, wake: &str, n: usize, fpm: f64, desc_fpm: f64) {
        for _ in 0..n {
            acc.add_flight(ty, wake, &flight(fpm, desc_fpm));
        }
    }

    #[test]
    fn a_family_is_profiled_at_twenty_pooled_flights_when_no_type_is() {
        assert_eq!(MIN_FLIGHTS_PER_FIELD, 20);
        let mut acc = Accumulator::default();
        add(&mut acc, "CRJ2", "", 10, 2_000.0, 2_000.0);
        add(&mut acc, "E75L", "", 10, 2_100.0, 2_000.0);
        let set = acc.finish(source(), engine_of);
        assert!(!set.types["CRJ2"].profiled && !set.types["E75L"].profiled);
        assert_eq!(set.families.keys().collect::<Vec<_>>(), ["jet"]);
        let jet = &set.families["jet"];
        assert_eq!(jet.flights, 20);
        assert!(jet.profiled);
        assert_eq!(jet.fields["desc_fpm"].flights, 20);
        assert_eq!(jet.fields["desc_fpm"].median, 2_000.0);
        assert_eq!(jet.fields["climb_fpm_hi"].median, 2_050.0);
    }

    #[test]
    fn a_family_one_flight_short_of_the_threshold_is_not_profiled() {
        let mut acc = Accumulator::default();
        add(&mut acc, "CRJ2", "", 10, 2_000.0, 2_000.0);
        add(&mut acc, "E75L", "", 9, 2_100.0, 2_000.0);
        let set = acc.finish(source(), engine_of);
        let jet = &set.families["jet"];
        assert_eq!(jet.flights, 19);
        assert!(!jet.profiled);
        assert!(!jet.fields["desc_fpm"].measured());
        assert_eq!(
            jet.apply_to(&AircraftProfile::default()),
            AircraftProfile::default()
        );
    }

    #[test]
    fn a_profiled_type_still_pools_its_flights_into_its_family() {
        let mut acc = Accumulator::default();
        add(&mut acc, "B738", "", 20, 3_000.0, 2_000.0);
        add(&mut acc, "CRJ2", "", 5, 1_000.0, 2_000.0);
        let set = acc.finish(source(), engine_of);
        assert!(set.types["B738"].profiled);
        assert_eq!(set.families["jet"].flights, 25);
        assert_eq!(set.families["jet"].fields["climb_fpm_hi"].flights, 25);
    }

    #[test]
    fn a_type_with_no_engine_family_pools_into_no_family() {
        let mut acc = Accumulator::default();
        add(&mut acc, "QQQQ", "", 20, 2_000.0, 2_000.0);
        add(&mut acc, "C172", "", 20, 700.0, 600.0);
        let set = acc.finish(source(), engine_of);
        assert_eq!(set.families.keys().collect::<Vec<_>>(), ["piston"]);
        assert_eq!(set.families["piston"].flights, 20);
        assert_eq!(set.types["QQQQ"].engine, None);
    }

    #[test]
    fn wake_pools_key_on_each_flights_own_wake_and_a_blank_wake_pools_nowhere() {
        let mut acc = Accumulator::default();
        add(&mut acc, "B77W", "H", 3, 1_500.0, 2_000.0);
        add(&mut acc, "B77W", "j", 1, 1_800.0, 2_000.0);
        add(&mut acc, "B77W", "", 2, 9_000.0, 2_000.0);
        let set = acc.finish(source(), engine_of);
        assert_eq!(set.types["B77W"].wake.as_deref(), Some("H"));
        assert_eq!(set.wakes.keys().collect::<Vec<_>>(), ["H", "J"]);
        assert_eq!(set.wakes["H"].flights, 3);
        assert_eq!(set.wakes["J"].flights, 1);
        assert_eq!(set.wakes["H"].fields["climb_fpm_hi"].q3, 1_500.0);
    }

    #[test]
    fn a_wake_pool_is_profiled_only_at_the_threshold() {
        let mut acc = Accumulator::default();
        add(&mut acc, "B77W", "H", 11, 1_500.0, 2_000.0);
        add(&mut acc, "A359", "H", 9, 1_500.0, 2_000.0);
        add(&mut acc, "C172", "L", 10, 700.0, 600.0);
        add(&mut acc, "PA28", "L", 9, 700.0, 600.0);
        let set = acc.finish(source(), |_| None);
        assert_eq!(
            (set.wakes["H"].flights, set.wakes["H"].profiled),
            (20, true)
        );
        assert_eq!(
            (set.wakes["L"].flights, set.wakes["L"].profiled),
            (19, false)
        );
    }

    #[test]
    fn two_families_with_different_values_each_land_in_their_own_row() {
        let mut acc = Accumulator::default();
        add(&mut acc, "CRJ2", "", 20, 2_000.0, 2_400.0);
        for _ in 0..20 {
            acc.add_flight(
                "AT72",
                "",
                &[
                    (Field::ClimbFpmHi, 1_200.0),
                    (Field::CruiseTas, 270.0),
                    (Field::DescFpm, 1_500.0),
                ],
            );
        }
        let set = acc.finish(source(), engine_of);
        assert_eq!(set.families["jet"].fields["desc_fpm"].median, 2_400.0);
        assert_eq!(set.families["turboprop"].fields["desc_fpm"].median, 1_500.0);
        assert_eq!(set.families["turboprop"].fields["cruise_tas"].q3, 270.0);
        assert_eq!(set.families["jet"].flights, 20);
        assert_eq!(set.families["turboprop"].flights, 20);
    }

    #[test]
    fn a_pooled_row_applies_to_a_base_exactly_as_a_type_row_does() {
        let stat = |k: f64, flights: usize| FieldStat {
            median: k,
            q1: k - 0.5,
            q3: k + 0.5,
            flights,
        };
        let mut fields: BTreeMap<String, FieldStat> = Field::ALL
            .iter()
            .enumerate()
            .map(|(i, f)| (f.name().to_string(), stat(i as f64 + 1.0, 20)))
            .collect();
        fields.insert("desc_ias_lo".into(), stat(99.0, 19));
        let base = AircraftProfile {
            climb_ias_lo: 201.0,
            climb_ias_hi: 202.0,
            climb_mach: Some(0.71),
            climb_fpm_lo: 2_001.0,
            climb_fpm_hi: 2_002.0,
            cruise_tas: Some(401.0),
            cruise_mach: Some(0.72),
            service_ceiling_ft: 40_001.0,
            desc_mach: Some(0.73),
            desc_ias_hi: 203.0,
            desc_ias_lo: 204.0,
            desc_fpm: 2_003.0,
        };
        let pooled = PooledProfile {
            flights: 20,
            profiled: true,
            fields: fields.clone(),
        };
        let typed = TypeProfile {
            flights: 20,
            wake: None,
            engine: None,
            profiled: true,
            fields,
        };
        let p = pooled.apply_to(&base);
        assert_eq!(p, typed.apply_to(&base));
        let k = |f: Field| Field::ALL.iter().position(|g| *g == f).unwrap() as f64 + 1.0;
        assert_eq!(
            p.cruise_tas,
            Some(k(Field::CruiseTas) + 0.5),
            "a cap takes q3"
        );
        assert_eq!(p.desc_fpm, k(Field::DescFpm), "a schedule takes the median");
        assert_eq!(p.desc_ias_lo, 204.0, "19 flights leave the base value");
        assert_eq!(p.service_ceiling_ft, 40_001.0);
    }

    #[test]
    fn families_and_wakes_round_trip_through_the_output_json() {
        let mut acc = Accumulator::default();
        add(&mut acc, "CRJ2", "M", 20, 2_000.0, 2_000.0);
        let set = acc.finish(source(), engine_of);
        let json = serde_json::to_value(&set).unwrap();
        assert_eq!(json["families"]["jet"]["flights"], 20);
        assert_eq!(json["families"]["jet"]["profiled"], true);
        assert_eq!(json["wakes"]["M"]["flights"], 20);
        let back: ProfileSet = serde_json::from_value(json).unwrap();
        assert_eq!(back, set);
        assert_eq!(back.check(), Ok(()));
    }

    #[test]
    fn a_file_written_before_family_pooling_deserializes_and_passes_check() {
        let old = r#"{
            "source": {"from": "a", "to": "b", "flights_read": 20, "training_flights": 20,
                       "held_out_flights": 0, "held_out_rule": "t"},
            "min_flights_per_field": 20,
            "types": {"B738": {"flights": 20, "wake": null, "engine": "jet", "profiled": true,
                "fields": {"desc_fpm": {"median": 2000.0, "q1": 1900.0, "q3": 2100.0, "flights": 20}}}}
        }"#;
        let set: ProfileSet = serde_json::from_str(old).unwrap();
        assert!(set.families.is_empty() && set.wakes.is_empty());
        assert_eq!(set.check(), Ok(()));
        assert_eq!(set.types["B738"].fields["desc_fpm"].median, 2_000.0);
    }

    #[test]
    fn check_refuses_an_unknown_field_in_a_family_or_wake_row() {
        let mut acc = Accumulator::default();
        add(&mut acc, "CRJ2", "M", 20, 2_000.0, 2_000.0);
        let set = acc.finish(source(), engine_of);
        assert_eq!(set.check(), Ok(()));
        let stat = set.families["jet"].fields["desc_fpm"].clone();

        let mut family = set.clone();
        let jet = family.families.get_mut("jet").unwrap();
        jet.fields.insert("desc_fmp".into(), stat.clone());
        assert_eq!(
            family.check(),
            Err("family:jet: unknown field \"desc_fmp\"".into())
        );

        let mut wake = set.clone();
        let m = wake.wakes.get_mut("M").unwrap();
        m.fields.insert("desc_fmp".into(), stat);
        assert_eq!(
            wake.check(),
            Err("wake:M: unknown field \"desc_fmp\"".into())
        );
    }
}
