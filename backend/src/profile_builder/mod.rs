//! Per-type aircraft performance measured from the stats history, and a backtest that scores a
//! profile set against held-out flights (#828). Run offline by the `aircraft-profile-builder` bin;
//! nothing here runs inside the server.
//!
//! ```text
//! stats.flight + stats.position + stats.winds
//!   ─▶ build:    training flights ─▶ samples::flight_samples ─▶ stats::Accumulator ─▶ ProfileSet
//!   ─▶ backtest: held-out flights ─▶ backtest::predict_case (predict::arrival_eta, arrival_route + eta_along_route)
//!                with set A (flow.aircraft_profile) and set B (A + the ProfileSet) ─▶ ETA error
//! ```
//!
//! The split between training and held-out flights is [`is_held_out`], shared by both halves so a
//! flight that shaped a profile is never used to score it.

pub mod backtest;
pub mod samples;
pub mod stats;

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    feed::{flow::engine_family, vatsim::FlightPlan},
    repos::profile_history::{self, HistFlight},
};
use samples::{WindsTimeline, flight_samples};
use stats::{Accumulator, ProfileSet, Source};

/// One flight in five is held out for the backtest. `session_id` is an FNV hash of the CID and logon
/// time (`0039_stats.sql`), so its residue is an even, repeatable split.
pub fn is_held_out(session_id: i64) -> bool {
    session_id.rem_euclid(5) == 0
}

/// [`is_held_out`] in words, for the output file.
pub const HELD_OUT_RULE: &str = "session_id mod 5 == 0";

/// The ICAO type and wake class of a stored flight, through the same parser the live feed uses
/// (`FlightPlan::aircraft_type_wake`). `stats.flight` does not store the full equipment string the
/// live feed reads the wake from, so `aircraft_faa` stands in for it: its leading segment carries
/// `H/` or `J/` for a heavy or super and nothing for the rest.
pub fn type_and_wake(f: &HistFlight) -> (String, String) {
    FlightPlan {
        aircraft: f.aircraft_faa.clone().unwrap_or_default(),
        aircraft_short: f.aircraft_short.clone().unwrap_or_default(),
        aircraft_faa: f.aircraft_faa.clone().unwrap_or_default(),
        ..Default::default()
    }
    .aircraft_type_wake()
}

/// Flights read per batch: each batch's positions (and the backtest's plan revisions) come in one
/// query, and its winds in another covering the batch's span. Flights arrive in `first_seen` order,
/// so a batch spans a short stretch of time and holds few winds snapshots in memory.
pub const FLIGHTS_PER_BATCH: usize = 500;

/// The time a batch of flights covers: the earliest `first_seen` to the latest `last_seen`.
pub(crate) fn batch_span<'a>(
    batch: impl Iterator<Item = &'a HistFlight>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    batch.fold(
        (DateTime::<Utc>::MAX_UTC, DateTime::<Utc>::MIN_UTC),
        |(lo, hi), f| (lo.min(f.first_seen), hi.max(f.last_seen)),
    )
}

/// Measure every type from the training flights wholly inside `[from, to)`.
pub async fn build(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<ProfileSet, ApiError> {
    let flights = profile_history::flights(pool, from, to).await?;
    let training: Vec<&HistFlight> = flights
        .iter()
        .filter(|f| !is_held_out(f.session_id))
        .collect();
    let mut acc = Accumulator::default();
    for batch in training.chunks(FLIGHTS_PER_BATCH) {
        let ids: Vec<i64> = batch.iter().map(|f| f.session_id).collect();
        let positions = profile_history::positions(pool, &ids).await?;
        let (start, end) = batch_span(batch.iter().copied());
        let winds = profile_history::winds_covering(pool, start, end).await?;
        let timeline = WindsTimeline(&winds);
        for f in batch {
            let pos = positions.get(&f.session_id).map_or(&[][..], Vec::as_slice);
            let samples = flight_samples(pos, &timeline);
            let (ty, wake) = type_and_wake(f);
            acc.add_flight(&ty, &wake, &samples.per_flight_medians());
        }
    }
    let source = Source {
        from: from.to_rfc3339_opts(SecondsFormat::Secs, true),
        to: to.to_rfc3339_opts(SecondsFormat::Secs, true),
        flights_read: flights.len(),
        training_flights: training.len(),
        held_out_flights: flights.len() - training.len(),
        held_out_rule: HELD_OUT_RULE.to_string(),
    };
    Ok(acc.finish(source, engine_family))
}

/// The output file's bytes: pretty JSON with a trailing newline. Every map is a `BTreeMap` and
/// every number is rounded, so the same data always serializes to the same bytes.
pub fn to_json(set: &ProfileSet) -> String {
    let mut s = serde_json::to_string_pretty(set).expect("a ProfileSet always serializes");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests;
