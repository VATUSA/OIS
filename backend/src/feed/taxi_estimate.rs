//! Robust per-(gate, aircraft type, runway) pushback, start-up, and taxi-out estimates from
//! sub-issue C's raw observations (`stats.taxi_observation`, #164 sub-issue D). Pure, DB-free math
//! — `repos::stats::taxi_samples_for_airport` fetches the sample set this operates over. No
//! trajectory/ETA wiring here; that's sub-issue E.

use super::predict::GROUND_TAXI_SEC;

/// One persisted observation. `repos::stats::taxi_samples_for_airport` queries straight into this
/// (the `FromRow` derive is harmless for unit-testing this module — nothing here needs a live DB).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaxiSample {
    pub gate_id: Option<String>,
    pub aircraft: Option<String>,
    pub runway: Option<String>,
    /// `None` means the departure did not push back — a no-tug gate-out, powerback or GA departure
    /// (#277). Every stored row comes from a completed phase machine (one first seen already moving
    /// is never recorded), so this counts as a real zero in the estimate, not a missing value (#287).
    pub pushback_sec: Option<i32>,
    /// `None` for the same reason as [`Self::pushback_sec`]: no push, so no start-up gap after one.
    pub startup_sec: Option<i32>,
    pub taxi_sec: i32,
}

/// Which ladder rung produced an estimate — least to most generic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateTier {
    GateTypeRunway,
    AirportRunway,
    Airport,
    Default,
}

impl EstimateTier {
    /// Stable wire/query string for this tier (#183's staff insights view filters on these).
    pub fn as_str(self) -> &'static str {
        match self {
            EstimateTier::GateTypeRunway => "gate_type_runway",
            EstimateTier::AirportRunway => "airport_runway",
            EstimateTier::Airport => "airport",
            EstimateTier::Default => "default",
        }
    }

    /// Inverse of [`Self::as_str`], for parsing a filter query param.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "gate_type_runway" => Some(Self::GateTypeRunway),
            "airport_runway" => Some(Self::AirportRunway),
            "airport" => Some(Self::Airport),
            "default" => Some(Self::Default),
            _ => None,
        }
    }

    /// A human-readable label for debug-mode surfaces (#164 sub-issue F) — not `Serialize` itself
    /// since only this label, not the enum, needs to cross the API boundary.
    pub fn label(self) -> &'static str {
        match self {
            EstimateTier::GateTypeRunway => "gate+type+runway",
            EstimateTier::AirportRunway => "airport+runway",
            EstimateTier::Airport => "airport",
            EstimateTier::Default => "default",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MetricEstimate {
    pub value_sec: f64,
    pub tier: EstimateTier,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct TaxiEstimate {
    pub pushback: MetricEstimate,
    pub startup: MetricEstimate,
    pub taxi: MetricEstimate,
}

/// Fewest values a ladder tier needs before its median is trusted; below this, fall to the next
/// (more general) tier.
const MIN_SAMPLES: usize = 5;

/// Absolute backstop clamp applied to a tier's median — independent of the median's own outlier
/// resistance, this catches a whole bucket being systematically bad (e.g. every sample at one gate
/// idling unusually long for some unmodeled reason) rather than trusting an implausible estimate.
pub(crate) const TAXI_BOUNDS_SEC: (f64, f64) = (60.0, 1800.0); // 1-30 min
pub(crate) const PUSHBACK_BOUNDS_SEC: (f64, f64) = (0.0, 1200.0); // 0-20 min
pub(crate) const STARTUP_BOUNDS_SEC: (f64, f64) = (0.0, 900.0); // 0-15 min

/// Defaults for the push and the start-up gap after it (#277). They split the original 5-minute
/// pushback+startup default so a no-data airport's total ground allowance is unchanged; adjust once
/// real data accumulates.
const DEFAULT_PUSHBACK_SEC: f64 = 180.0;
const DEFAULT_STARTUP_SEC: f64 = 120.0;

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

fn clamp(value: f64, (min, max): (f64, f64)) -> f64 {
    value.clamp(min, max)
}

/// Walk the fallback ladder for one metric (`extract` pulls that metric's value out of a sample,
/// `None` only when the sample can't speak to it at all), returning the first tier with
/// `>= MIN_SAMPLES` values, median-and-bounds-clamped, or `default` if even the airport-wide tier is
/// too sparse. A no-push departure is not silent: its pushback/start-up count as `0.0` (#287), so an
/// airport whose aircraft never push back learns that instead of falling back to the flat default.
fn estimate_metric(
    samples: &[TaxiSample],
    gate_id: Option<&str>,
    aircraft: Option<&str>,
    runway: Option<&str>,
    extract: impl Fn(&TaxiSample) -> Option<f64>,
    bounds: (f64, f64),
    default: f64,
) -> MetricEstimate {
    let tiered = |values: Vec<f64>, tier: EstimateTier| {
        let count = values.len();
        (count >= MIN_SAMPLES).then(|| MetricEstimate {
            value_sec: clamp(median(values), bounds),
            tier,
            sample_count: count,
        })
    };

    if let (Some(g), Some(a), Some(r)) = (gate_id, aircraft, runway) {
        let values: Vec<f64> = samples
            .iter()
            .filter(|s| {
                s.gate_id.as_deref() == Some(g)
                    && s.aircraft.as_deref() == Some(a)
                    && s.runway.as_deref() == Some(r)
            })
            .filter_map(&extract)
            .collect();
        if let Some(e) = tiered(values, EstimateTier::GateTypeRunway) {
            return e;
        }
    }

    if let Some(r) = runway {
        let values: Vec<f64> = samples
            .iter()
            .filter(|s| s.runway.as_deref() == Some(r))
            .filter_map(&extract)
            .collect();
        if let Some(e) = tiered(values, EstimateTier::AirportRunway) {
            return e;
        }
    }

    let values: Vec<f64> = samples.iter().filter_map(&extract).collect();
    if let Some(e) = tiered(values, EstimateTier::Airport) {
        return e;
    }

    MetricEstimate {
        value_sec: default,
        tier: EstimateTier::Default,
        sample_count: 0,
    }
}

/// A robust pushback, start-up, and taxi-out estimate for a departure matching `gate_id`/`aircraft`/
/// `runway` (each `None` skips straight past the tiers that need it). `samples` must already be
/// scoped to one airport (`repos::stats::taxi_samples_for_airport`) — the broadest ladder tier is
/// simply "every sample given," not a separate airport filter.
pub fn estimate(
    samples: &[TaxiSample],
    gate_id: Option<&str>,
    aircraft: Option<&str>,
    runway: Option<&str>,
) -> TaxiEstimate {
    TaxiEstimate {
        pushback: estimate_metric(
            samples,
            gate_id,
            aircraft,
            runway,
            |s| Some(s.pushback_sec.unwrap_or(0) as f64),
            PUSHBACK_BOUNDS_SEC,
            DEFAULT_PUSHBACK_SEC,
        ),
        startup: estimate_metric(
            samples,
            gate_id,
            aircraft,
            runway,
            |s| Some(s.startup_sec.unwrap_or(0) as f64),
            STARTUP_BOUNDS_SEC,
            DEFAULT_STARTUP_SEC,
        ),
        taxi: estimate_metric(
            samples,
            gate_id,
            aircraft,
            runway,
            |s| Some(s.taxi_sec as f64),
            TAXI_BOUNDS_SEC,
            GROUND_TAXI_SEC,
        ),
    }
}

/// The three phases summed into the single ground allowance the prediction service adds to a
/// not-yet-airborne flight's time (#164 sub-issue E — `feed::predict::eta_along_route`).
pub fn ground_allowance_sec(est: &TaxiEstimate) -> f64 {
    est.pushback.value_sec + est.startup.value_sec + est.taxi.value_sec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_as_str_and_parse_round_trip() {
        for tier in [
            EstimateTier::GateTypeRunway,
            EstimateTier::AirportRunway,
            EstimateTier::Airport,
            EstimateTier::Default,
        ] {
            assert_eq!(EstimateTier::parse(tier.as_str()), Some(tier));
        }
        assert_eq!(EstimateTier::parse("not-a-tier"), None);
    }

    fn sample(
        gate: &str,
        aircraft: &str,
        runway: &str,
        pushback: Option<i32>,
        taxi: i32,
    ) -> TaxiSample {
        TaxiSample {
            gate_id: Some(gate.to_string()),
            aircraft: Some(aircraft.to_string()),
            runway: Some(runway.to_string()),
            pushback_sec: pushback,
            startup_sec: pushback.map(|_| 90),
            taxi_sec: taxi,
        }
    }

    #[test]
    fn median_rejects_a_few_extreme_outliers() {
        // 10 normal samples around 90s + 2 extreme outliers in the same bucket. A mean would be
        // pulled toward ~300s; the median should stay near the normal cluster.
        let mut samples: Vec<TaxiSample> = (0..10)
            .map(|i| sample("A1", "B738", "27L", Some(60), 85 + i))
            .collect();
        samples.push(sample("A1", "B738", "27L", Some(60), 1700));
        samples.push(sample("A1", "B738", "27L", Some(60), 1800));

        let est = estimate(&samples, Some("A1"), Some("B738"), Some("27L"));
        assert_eq!(est.taxi.tier, EstimateTier::GateTypeRunway);
        assert!(
            (85.0..=95.0).contains(&est.taxi.value_sec),
            "median should stay near the normal cluster, got {}",
            est.taxi.value_sec
        );
    }

    #[test]
    fn most_specific_tier_wins_when_all_qualify() {
        let mut samples: Vec<TaxiSample> = (0..MIN_SAMPLES)
            .map(|_| sample("A1", "B738", "27L", Some(60), 90))
            .collect();
        // A different gate/type sharing the same runway — qualifies for the airport-runway tier,
        // but shouldn't be picked over the exact gate/type/runway match above.
        samples.extend((0..MIN_SAMPLES).map(|_| sample("A2", "A320", "27L", Some(90), 150)));
        // A different runway entirely — qualifies for the airport-wide tier only.
        samples.extend((0..MIN_SAMPLES).map(|_| sample("A3", "A320", "09R", Some(120), 300)));

        let est = estimate(&samples, Some("A1"), Some("B738"), Some("27L"));
        assert_eq!(est.taxi.tier, EstimateTier::GateTypeRunway);
        assert_eq!(est.taxi.value_sec, 90.0);
    }

    #[test]
    fn falls_back_exactly_one_tier_when_the_specific_key_is_sparse() {
        // Only 2 samples for the exact gate/type/runway (< MIN_SAMPLES) ...
        let mut samples = vec![
            sample("A1", "B738", "27L", Some(60), 90),
            sample("A1", "B738", "27L", Some(60), 95),
        ];
        // ... but enough for the airport-runway tier (same runway, different gate/type).
        samples.extend((0..MIN_SAMPLES).map(|_| sample("A2", "A320", "27L", Some(90), 200)));

        let est = estimate(&samples, Some("A1"), Some("B738"), Some("27L"));
        assert_eq!(est.taxi.tier, EstimateTier::AirportRunway);
        assert_eq!(est.taxi.value_sec, 200.0);
    }

    #[test]
    fn defaults_to_the_flat_8_minute_constant_when_every_tier_is_sparse() {
        let samples = vec![sample("A1", "B738", "27L", None, 90)];
        let est = estimate(&samples, Some("A1"), Some("B738"), Some("27L"));
        assert_eq!(est.taxi.tier, EstimateTier::Default);
        assert_eq!(est.taxi.value_sec, GROUND_TAXI_SEC);
        assert_eq!(est.taxi.sample_count, 0);
        // No pushback figure on the lone sample either, so pushback also defaults.
        assert_eq!(est.pushback.tier, EstimateTier::Default);
        assert_eq!(est.pushback.value_sec, DEFAULT_PUSHBACK_SEC);
        assert_eq!(est.startup.tier, EstimateTier::Default);
        assert_eq!(est.startup.value_sec, DEFAULT_STARTUP_SEC);
    }

    #[test]
    fn pushback_and_taxi_walk_the_ladder_independently() {
        // One sample short of MIN_SAMPLES at the exact key for pushback (the tier needs 5), but the
        // runway tier has plenty — pushback falls back on its own while taxi stays specific. NULLs
        // no longer thin a pool (#287), so the fallback is forced by count, not by missing values.
        let mut samples: Vec<TaxiSample> = (0..MIN_SAMPLES - 1)
            .map(|i| sample("A1", "B738", "27L", Some(40), 85 + i as i32))
            .collect();
        samples.extend((0..MIN_SAMPLES).map(|_| sample("A2", "A320", "27L", Some(70), 200)));

        let est = estimate(&samples, Some("A1"), Some("B738"), Some("27L"));
        assert_eq!(est.taxi.tier, EstimateTier::AirportRunway);
        assert_eq!(est.pushback.tier, EstimateTier::AirportRunway);
        assert_eq!(est.pushback.value_sec, 70.0);
    }

    #[test]
    fn startup_walks_the_ladder_independently_and_sums_into_the_allowance() {
        // Every metric resolves at the exact key here; the allowance is the three medians summed.
        let samples: Vec<TaxiSample> = (0..MIN_SAMPLES)
            .map(|_| sample("A1", "B738", "27L", Some(60), 200))
            .collect();

        let est = estimate(&samples, Some("A1"), Some("B738"), Some("27L"));
        assert_eq!(est.pushback.tier, EstimateTier::GateTypeRunway);
        assert_eq!(est.startup.tier, EstimateTier::GateTypeRunway);
        assert_eq!(est.startup.value_sec, 90.0);
        assert_eq!(ground_allowance_sec(&est), 60.0 + 90.0 + 200.0);
    }

    /// #287: a no-tug field's observations all carry NULL push/start-up. Dropping them left the
    /// estimator on its flat defaults forever, adding ~300 s of ground time nobody spends.
    #[test]
    fn an_airport_where_nothing_pushes_back_learns_zero_not_the_default() {
        let samples: Vec<TaxiSample> = (0..MIN_SAMPLES + 3)
            .map(|i| TaxiSample {
                pushback_sec: None,
                startup_sec: None,
                ..sample("A1", "C172", "27L", None, 200 + i as i32)
            })
            .collect();

        let est = estimate(&samples, Some("A1"), Some("C172"), Some("27L"));
        assert_eq!(est.pushback.tier, EstimateTier::GateTypeRunway);
        assert_eq!(est.pushback.value_sec, 0.0);
        assert_eq!(est.startup.tier, EstimateTier::GateTypeRunway);
        assert_eq!(est.startup.value_sec, 0.0);
        // The whole ground allowance is the taxi figure — no phantom push or start-up.
        assert_eq!(ground_allowance_sec(&est), est.taxi.value_sec);
    }

    /// #287: a GA-heavy field where a handful of airliners do push. The majority answer wins the
    /// median instead of the pushing minority being the only voice in the pool.
    #[test]
    fn a_no_push_majority_outvotes_the_pushing_minority() {
        let mut samples: Vec<TaxiSample> = (0..9)
            .map(|_| TaxiSample {
                pushback_sec: None,
                startup_sec: None,
                ..sample("A1", "C172", "27L", None, 200)
            })
            .collect();
        samples.extend((0..5).map(|_| sample("A1", "C172", "27L", Some(150), 400)));

        let est = estimate(&samples, Some("A1"), Some("C172"), Some("27L"));
        assert_eq!(est.pushback.value_sec, 0.0);
        assert_eq!(est.startup.value_sec, 0.0);
        assert_eq!(
            est.pushback.sample_count, 14,
            "every observation counts, NULL included"
        );
        // Taxi is untouched: it's never NULL, and its median still spans both groups.
        assert_eq!(est.taxi.value_sec, 200.0);
    }
}
