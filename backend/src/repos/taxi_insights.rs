//! Staff-browsable history over raw taxi/pushback observations (`stats.taxi_observation`, #164
//! sub-issue C) and their derived per-(gate,aircraft,runway) estimates (sub-issue D) — #183.
//! Read-only; the only interaction with the live estimator (`feed::taxi_estimate`) is calling its
//! pure `estimate()` fn against a filtered sample pool fetched here.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::feed::taxi_estimate::{
    self, EstimateTier, PUSHBACK_BOUNDS_SEC, TAXI_BOUNDS_SEC, TaxiSample,
};
use crate::models::{TaxiEstimateEntry, TaxiObservationEntry};

fn db(e: sqlx::Error) -> ApiError {
    tracing::warn!(error = %e, "taxi_insights db error");
    ApiError::Internal
}

/// A row is an "outlier" when it falls outside `taxi_estimate`'s own sanity-clamp bounds — the
/// same bounds the estimator already applies to a tier's median. Computed at query time (formatted
/// in as literals since the bounds are `const`, not persisted or bound as params), never stored.
fn outlier_expr() -> String {
    format!(
        "(taxi_sec < {tmin} or taxi_sec > {tmax} \
          or (pushback_sec is not null and (pushback_sec < {pmin} or pushback_sec > {pmax})))",
        tmin = TAXI_BOUNDS_SEC.0,
        tmax = TAXI_BOUNDS_SEC.1,
        pmin = PUSHBACK_BOUNDS_SEC.0,
        pmax = PUSHBACK_BOUNDS_SEC.1,
    )
}

/// Shared `$1..$6` dimension/time-range filter, reused by the observation count and fetch queries.
fn obs_where() -> &'static str {
    "($1::text is null or airport = $1) \
     and ($2::text is null or gate_id = $2) \
     and ($3::text is null or aircraft = $3) \
     and ($4::text is null or runway = $4) \
     and ($5::timestamptz is null or observed_at >= $5) \
     and ($6::timestamptz is null or observed_at <= $6)"
}

pub struct ObservationFilters {
    pub airport: Option<String>,
    pub gate_id: Option<String>,
    pub aircraft: Option<String>,
    pub runway: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub include_outliers: bool,
    pub limit: i64,
    pub offset: i64,
}

pub async fn count_taxi_observations(
    pool: &PgPool,
    f: &ObservationFilters,
) -> Result<i64, ApiError> {
    let outlier_clause = if f.include_outliers {
        String::new()
    } else {
        format!(" and not {}", outlier_expr())
    };
    let sql = format!(
        "select count(*) from stats.taxi_observation where {}{outlier_clause}",
        obs_where()
    );
    sqlx::query_scalar::<_, i64>(&sql)
        .bind(&f.airport)
        .bind(&f.gate_id)
        .bind(&f.aircraft)
        .bind(&f.runway)
        .bind(f.from)
        .bind(f.to)
        .fetch_one(pool)
        .await
        .map_err(db)
}

pub async fn fetch_taxi_observations(
    pool: &PgPool,
    f: &ObservationFilters,
) -> Result<Vec<TaxiObservationEntry>, ApiError> {
    let outlier_clause = if f.include_outliers {
        String::new()
    } else {
        format!(" and not {}", outlier_expr())
    };
    let sql = format!(
        "select id, airport, gate_id, aircraft, runway, pushback_sec, taxi_sec, observed_at, \
                {expr} as is_outlier \
         from stats.taxi_observation where {base}{outlier_clause} \
         order by observed_at desc limit $7 offset $8",
        expr = outlier_expr(),
        base = obs_where(),
    );
    sqlx::query_as::<_, TaxiObservationEntry>(&sql)
        .bind(&f.airport)
        .bind(&f.gate_id)
        .bind(&f.aircraft)
        .bind(&f.runway)
        .bind(f.from)
        .bind(f.to)
        .bind(f.limit)
        .bind(f.offset)
        .fetch_all(pool)
        .await
        .map_err(db)
}

pub struct EstimateFilters {
    pub airport: String,
    pub gate_id: Option<String>,
    pub aircraft: Option<String>,
    pub runway: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub include_outliers: bool,
    pub fallback_tier: Option<EstimateTier>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(sqlx::FromRow)]
struct ComboRow {
    gate_id: Option<String>,
    aircraft: Option<String>,
    runway: Option<String>,
}

/// The airport's full sample pool for `estimate()`'s ladder (time-range + outlier scoped, but NOT
/// gate/aircraft/runway scoped — those only pick which *combos* to compute below; the broader
/// ladder tiers need the unrestricted pool, per `estimate()`'s own contract).
async fn sample_pool(
    pool: &PgPool,
    airport: &str,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    include_outliers: bool,
) -> Result<Vec<TaxiSample>, ApiError> {
    let outlier_clause = if include_outliers {
        String::new()
    } else {
        format!(" and not {}", outlier_expr())
    };
    let sql = format!(
        "select gate_id, aircraft, runway, pushback_sec, taxi_sec from stats.taxi_observation \
         where airport = $1 and ($2::timestamptz is null or observed_at >= $2) \
           and ($3::timestamptz is null or observed_at <= $3){outlier_clause}"
    );
    sqlx::query_as(&sql)
        .bind(airport)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
        .map_err(db)
}

/// Distinct (gate_id, aircraft, runway) combos present for this airport, matching the combo-level
/// filters. Capped at 2000 — one airport's cardinality (gates × types × runways) is bounded, so
/// this avoids an unbounded fetch without needing SQL-level pagination over combos.
async fn distinct_combos(pool: &PgPool, f: &EstimateFilters) -> Result<Vec<ComboRow>, ApiError> {
    sqlx::query_as::<_, ComboRow>(
        "select gate_id, aircraft, runway from stats.taxi_observation \
         where airport = $1 \
           and ($2::text is null or gate_id = $2) and ($3::text is null or aircraft = $3) \
           and ($4::text is null or runway = $4) \
           and ($5::timestamptz is null or observed_at >= $5) \
           and ($6::timestamptz is null or observed_at <= $6) \
         group by gate_id, aircraft, runway order by count(*) desc, gate_id, aircraft, runway \
         limit 2000",
    )
    .bind(&f.airport)
    .bind(&f.gate_id)
    .bind(&f.aircraft)
    .bind(&f.runway)
    .bind(f.from)
    .bind(f.to)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Computes one `TaxiEstimate` per distinct combo present for `f.airport`, filters by
/// `fallback_tier` (matching either metric), then paginates in-memory. Returns `(page, total)`.
pub async fn fetch_taxi_estimates(
    pool: &PgPool,
    f: &EstimateFilters,
) -> Result<(Vec<TaxiEstimateEntry>, i64), ApiError> {
    let combos = distinct_combos(pool, f).await?;
    let samples = sample_pool(pool, &f.airport, f.from, f.to, f.include_outliers).await?;

    let mut all: Vec<TaxiEstimateEntry> = combos
        .into_iter()
        .map(|c| {
            let est = taxi_estimate::estimate(
                &samples,
                c.gate_id.as_deref(),
                c.aircraft.as_deref(),
                c.runway.as_deref(),
            );
            TaxiEstimateEntry {
                airport: f.airport.clone(),
                gate_id: c.gate_id,
                aircraft: c.aircraft,
                runway: c.runway,
                pushback_sec: est.pushback.value_sec,
                pushback_tier: est.pushback.tier.as_str().to_string(),
                pushback_sample_count: est.pushback.sample_count as i64,
                taxi_sec: est.taxi.value_sec,
                taxi_tier: est.taxi.tier.as_str().to_string(),
                taxi_sample_count: est.taxi.sample_count as i64,
            }
        })
        .filter(|e| {
            f.fallback_tier
                .is_none_or(|t| e.taxi_tier == t.as_str() || e.pushback_tier == t.as_str())
        })
        .collect();

    let total = all.len() as i64;
    let start = f.offset.clamp(0, all.len() as i64) as usize;
    let end = (f.offset + f.limit).clamp(0, all.len() as i64) as usize;
    let page = if start < end {
        all.drain(start..end).collect()
    } else {
        Vec::new()
    };
    Ok((page, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos::stats::{TaxiObservationRow, insert_taxi_observations};
    use chrono::Duration;

    async fn seed_gate(pool: &PgPool, icao: &str) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into flow.airport_gate (icao, name, lat, lon) \
             values ($1, 'A1', 40.0, -74.0) returning id",
        )
        .bind(icao)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn row(
        airport: &str,
        gate_id: Option<&str>,
        aircraft: &str,
        runway: &str,
        pushback_sec: Option<i32>,
        taxi_sec: i32,
        observed_at: DateTime<Utc>,
    ) -> TaxiObservationRow {
        TaxiObservationRow {
            airport: airport.to_string(),
            gate_id: gate_id.map(str::to_string),
            aircraft: Some(aircraft.to_string()),
            runway: Some(runway.to_string()),
            pushback_sec,
            taxi_sec,
            observed_at,
        }
    }

    fn empty_obs_filters() -> ObservationFilters {
        ObservationFilters {
            airport: None,
            gate_id: None,
            aircraft: None,
            runway: None,
            from: None,
            to: None,
            include_outliers: true,
            limit: 100,
            offset: 0,
        }
    }

    #[sqlx::test]
    async fn observation_filters_narrow_independently_and_combined(pool: PgPool) {
        let now = Utc::now();
        let gate_a = seed_gate(&pool, "KAAA").await;
        let gate_b = seed_gate(&pool, "KBBB").await;
        insert_taxi_observations(
            &pool,
            &[
                row("KAAA", Some(&gate_a), "B738", "27L", Some(60), 300, now),
                row("KAAA", Some(&gate_b), "A320", "27L", Some(60), 300, now),
                row("KAAA", Some(&gate_a), "B738", "09R", Some(60), 300, now),
                row("KBBB", Some(&gate_a), "B738", "27L", Some(60), 300, now),
            ],
        )
        .await
        .unwrap();

        let by_airport = ObservationFilters {
            airport: Some("KAAA".into()),
            ..empty_obs_filters()
        };
        assert_eq!(
            count_taxi_observations(&pool, &by_airport).await.unwrap(),
            3
        );

        let by_gate = ObservationFilters {
            gate_id: Some(gate_a.clone()),
            ..empty_obs_filters()
        };
        assert_eq!(count_taxi_observations(&pool, &by_gate).await.unwrap(), 3);

        let by_aircraft = ObservationFilters {
            aircraft: Some("A320".into()),
            ..empty_obs_filters()
        };
        assert_eq!(
            count_taxi_observations(&pool, &by_aircraft).await.unwrap(),
            1
        );

        let by_runway = ObservationFilters {
            runway: Some("09R".into()),
            ..empty_obs_filters()
        };
        assert_eq!(count_taxi_observations(&pool, &by_runway).await.unwrap(), 1);

        let combined = ObservationFilters {
            airport: Some("KAAA".into()),
            gate_id: Some(gate_a.clone()),
            runway: Some("27L".into()),
            ..empty_obs_filters()
        };
        assert_eq!(count_taxi_observations(&pool, &combined).await.unwrap(), 1);

        let time_range = ObservationFilters {
            from: Some(now - Duration::minutes(1)),
            to: Some(now + Duration::minutes(1)),
            ..empty_obs_filters()
        };
        assert_eq!(
            count_taxi_observations(&pool, &time_range).await.unwrap(),
            4
        );
        let outside_range = ObservationFilters {
            from: Some(now + Duration::hours(1)),
            ..empty_obs_filters()
        };
        assert_eq!(
            count_taxi_observations(&pool, &outside_range)
                .await
                .unwrap(),
            0
        );
    }

    #[sqlx::test]
    async fn include_outliers_toggle_affects_count_and_fetch(pool: PgPool) {
        let now = Utc::now();
        insert_taxi_observations(
            &pool,
            &[
                // Normal.
                row("KAAA", None, "B738", "27L", Some(60), 300, now),
                // taxi_sec above TAXI_BOUNDS_SEC.1 (1800).
                row("KAAA", None, "B738", "27L", Some(60), 5000, now),
                // pushback_sec above PUSHBACK_BOUNDS_SEC.1 (1200).
                row("KAAA", None, "B738", "27L", Some(2000), 300, now),
            ],
        )
        .await
        .unwrap();

        let all = fetch_taxi_observations(&pool, &empty_obs_filters())
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
        let flagged: Vec<bool> = all.iter().map(|o| o.is_outlier).collect();
        assert_eq!(
            flagged.iter().filter(|&&f| f).count(),
            2,
            "flagged: {flagged:?}"
        );

        let excluding = ObservationFilters {
            include_outliers: false,
            ..empty_obs_filters()
        };
        assert_eq!(count_taxi_observations(&pool, &excluding).await.unwrap(), 1);
        let fetched = fetch_taxi_observations(&pool, &excluding).await.unwrap();
        assert_eq!(fetched.len(), 1);
        assert!(!fetched[0].is_outlier);
    }

    fn empty_est_filters(airport: &str) -> EstimateFilters {
        EstimateFilters {
            airport: airport.to_string(),
            gate_id: None,
            aircraft: None,
            runway: None,
            from: None,
            to: None,
            include_outliers: true,
            fallback_tier: None,
            limit: 100,
            offset: 0,
        }
    }

    #[sqlx::test]
    async fn fetch_taxi_estimates_matches_a_direct_estimate_call(pool: PgPool) {
        let now = Utc::now();
        let gate = seed_gate(&pool, "KAAA").await;
        let rows: Vec<TaxiObservationRow> = (0..6)
            .map(|i| row("KAAA", Some(&gate), "B738", "27L", Some(60), 200 + i, now))
            .collect();
        insert_taxi_observations(&pool, &rows).await.unwrap();

        let (page, total) = fetch_taxi_estimates(&pool, &empty_est_filters("KAAA"))
            .await
            .unwrap();
        assert_eq!(
            total, 1,
            "exactly one distinct (gate, aircraft, runway) combo"
        );
        assert_eq!(page.len(), 1);

        let samples: Vec<TaxiSample> = (0..6)
            .map(|i| TaxiSample {
                gate_id: Some(gate.clone()),
                aircraft: Some("B738".to_string()),
                runway: Some("27L".to_string()),
                pushback_sec: Some(60),
                taxi_sec: 200 + i,
            })
            .collect();
        let direct = taxi_estimate::estimate(&samples, Some(&gate), Some("B738"), Some("27L"));
        assert_eq!(page[0].taxi_tier, direct.taxi.tier.as_str());
        assert_eq!(page[0].taxi_sec, direct.taxi.value_sec);
        assert_eq!(page[0].taxi_sample_count, direct.taxi.sample_count as i64);
    }

    #[sqlx::test]
    async fn excluding_outliers_can_change_a_combos_tier(pool: PgPool) {
        let now = Utc::now();
        let gate = seed_gate(&pool, "KAAA").await;
        // Fewer than MIN_SAMPLES (5) in-bounds rows...
        let mut rows: Vec<TaxiObservationRow> = (0..3)
            .map(|i| row("KAAA", Some(&gate), "B738", "27L", Some(60), 200 + i, now))
            .collect();
        // ...plus enough out-of-bounds rows that including them pads the sample count to
        // MIN_SAMPLES and lets the gate/type/runway tier resolve.
        rows.extend((0..2).map(|_| row("KAAA", Some(&gate), "B738", "27L", Some(60), 5000, now)));
        insert_taxi_observations(&pool, &rows).await.unwrap();

        let (with_outliers, _) = fetch_taxi_estimates(&pool, &empty_est_filters("KAAA"))
            .await
            .unwrap();
        assert_eq!(
            with_outliers[0].taxi_tier,
            EstimateTier::GateTypeRunway.as_str()
        );

        let excluding = EstimateFilters {
            include_outliers: false,
            ..empty_est_filters("KAAA")
        };
        let (without_outliers, _) = fetch_taxi_estimates(&pool, &excluding).await.unwrap();
        assert_eq!(
            without_outliers[0].taxi_tier,
            EstimateTier::Default.as_str(),
            "excluding the 2 outlier rows should drop below MIN_SAMPLES and fall to Default"
        );
    }

    #[sqlx::test]
    async fn fallback_tier_filter_excludes_non_matching_combos(pool: PgPool) {
        let now = Utc::now();
        let gate = seed_gate(&pool, "KAAA").await;
        // Combo 1: enough samples to resolve at GateTypeRunway.
        let mut rows: Vec<TaxiObservationRow> = (0..5)
            .map(|i| row("KAAA", Some(&gate), "B738", "27L", Some(60), 200 + i, now))
            .collect();
        // Combo 2: a single sample of its own, too thin for GateTypeRunway/AirportRunway, but the
        // airport-wide pool (6 samples total, combo 1's 5 included) clears MIN_SAMPLES, so this
        // falls to the Airport tier rather than all the way to Default.
        rows.push(row("KAAA", Some(&gate), "A320", "09R", Some(60), 400, now));
        insert_taxi_observations(&pool, &rows).await.unwrap();

        let airport_tier_only = EstimateFilters {
            fallback_tier: Some(EstimateTier::Airport),
            ..empty_est_filters("KAAA")
        };
        let (page, total) = fetch_taxi_estimates(&pool, &airport_tier_only)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(page[0].aircraft.as_deref(), Some("A320"));
    }
}
