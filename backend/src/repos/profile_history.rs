//! Read-only access to the stats history the aircraft-profile builder and its backtest mine (#828):
//! completed flights, their positions, the flight-plan revisions, and the
//! winds snapshots around a flight. Nothing here writes.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{errors::ApiError, feed::winds::Winds};

/// A completed flight with a filed plan, as `stats.flight` stores it (the latest plan revision).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HistFlight {
    pub session_id: i64,
    pub departure: String,
    pub arrival: String,
    pub aircraft_short: Option<String>,
    pub aircraft_faa: Option<String>,
    pub cruise_tas: Option<i32>,
    pub cruise_alt: Option<i32>,
    pub route: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// One stored feed tick for a flight.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HistPosition {
    pub ts: DateTime<Utc>,
    pub lat: f32,
    pub lon: f32,
    pub altitude: i32,
    pub groundspeed: i16,
    pub heading: i16,
}

/// The parts of a flight-plan revision the backtest replays with.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HistPlan {
    pub effective_from: DateTime<Utc>,
    pub route: Option<String>,
    pub cruise_alt: Option<i32>,
}

/// Completed flights with a departure, an arrival and an aircraft type, wholly inside
/// `[from, to)`, ordered by `first_seen` then `session_id`: every run walks them in the same order,
/// and a run of consecutive flights spans a short stretch of time, so one winds read covers a batch.
pub async fn flights(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<HistFlight>, ApiError> {
    sqlx::query_as::<_, HistFlight>(
        "select session_id, departure, arrival, aircraft_short, aircraft_faa, cruise_tas, \
                cruise_alt, route, first_seen, last_seen \
         from stats.flight \
         where status = 'completed' and departure is not null and arrival is not null \
           and coalesce(aircraft_short, aircraft_faa) is not null \
           and first_seen >= $1 and last_seen < $2 \
         order by first_seen, session_id",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)
}

/// Every stored position of each flight in `session_ids`, oldest first, in one query. A flight with
/// no stored position is absent from the map.
pub async fn positions(
    pool: &PgPool,
    session_ids: &[i64],
) -> Result<HashMap<i64, Vec<HistPosition>>, ApiError> {
    let rows = sqlx::query_as::<_, (i64, DateTime<Utc>, f32, f32, i32, i16, i16)>(
        "select session_id, ts, lat, lon, altitude, groundspeed, heading from stats.position \
         where session_id = any($1) order by session_id, ts",
    )
    .bind(session_ids)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    let mut out: HashMap<i64, Vec<HistPosition>> = HashMap::new();
    for (sid, ts, lat, lon, altitude, groundspeed, heading) in rows {
        out.entry(sid).or_default().push(HistPosition {
            ts,
            lat,
            lon,
            altitude,
            groundspeed,
            heading,
        });
    }
    Ok(out)
}

/// Every flight-plan revision of each flight in `session_ids`, oldest first, in one query.
pub async fn plans(
    pool: &PgPool,
    session_ids: &[i64],
) -> Result<HashMap<i64, Vec<HistPlan>>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        session_id: i64,
        #[sqlx(flatten)]
        plan: HistPlan,
    }
    let rows = sqlx::query_as::<_, Row>(
        "select session_id, effective_from, route, cruise_alt from stats.flight_plan \
         where session_id = any($1) order by session_id, effective_from",
    )
    .bind(session_ids)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    let mut out: HashMap<i64, Vec<HistPlan>> = HashMap::new();
    for r in rows {
        out.entry(r.session_id).or_default().push(r.plan);
    }
    Ok(out)
}

/// The revision in force at `at`: the newest of `revisions` (oldest first) that took effect at or
/// before it, or None when none had yet.
pub fn plan_at(revisions: &[HistPlan], at: DateTime<Utc>) -> Option<&HistPlan> {
    let n = revisions.partition_point(|r| r.effective_from <= at);
    revisions.get(n.checked_sub(1)?)
}

/// The winds snapshots that cover `[from, to]`: every snapshot inside it plus the last one before
/// `from`, oldest first. A snapshot applies from its `ts` until the next one, the way
/// [`super::stats::winds_at`] picks one for replay.
pub async fn winds_covering(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<(DateTime<Utc>, Winds)>, ApiError> {
    let rows = sqlx::query_as::<_, (DateTime<Utc>, sqlx::types::Json<Winds>)>(
        "(select ts, data from stats.winds where ts < $1 order by ts desc limit 1) \
         union all \
         (select ts, data from stats.winds where ts >= $1 and ts <= $2) \
         order by ts",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().map(|(ts, w)| (ts, w.0)).collect())
}

fn db(e: sqlx::Error) -> ApiError {
    tracing::warn!(error = %e, "profile history db error");
    ApiError::Internal
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use sqlx::PgPool;

    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap()
    }

    /// One position of `session_id` `sec` seconds after t0 at `alt`.
    async fn at(pool: &PgPool, session_id: i64, sec: i64, alt: i32) {
        sqlx::query(
            "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, heading) \
             values ($1, $2, 40, -100, $3, 300, 90)",
        )
        .bind(session_id)
        .bind(t0() + Duration::seconds(sec))
        .bind(alt)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Flights 10 and 20 tick 5 s apart, inserted newest first: each flight gets its own rows,
    /// oldest first. Flight 30, interleaved but not asked for, and flight 40, asked for with no
    /// rows, are absent.
    #[sqlx::test]
    async fn positions_group_to_their_own_flight_and_read_only_those_asked_for(pool: PgPool) {
        for k in (0..3).rev() {
            at(&pool, 30, k * 10 + 2, 9_000).await;
            at(&pool, 20, k * 10 + 5, 5_000 + 100 * k as i32).await;
            at(&pool, 10, k * 10, 1_000 + 100 * k as i32).await;
        }
        let map = positions(&pool, &[10, 20, 40]).await.unwrap();
        let alts = |sid| map[&sid].iter().map(|p| p.altitude).collect::<Vec<_>>();
        let secs = |sid| {
            map[&sid]
                .iter()
                .map(|p| (p.ts - t0()).num_seconds())
                .collect::<Vec<_>>()
        };
        assert_eq!(alts(10), vec![1_000, 1_100, 1_200]);
        assert_eq!(secs(10), vec![0, 10, 20]);
        assert_eq!(alts(20), vec![5_000, 5_100, 5_200]);
        assert_eq!(secs(20), vec![5, 15, 25]);
        let mut keys: Vec<_> = map.keys().copied().collect();
        keys.sort();
        assert_eq!(keys, vec![10, 20]);
    }

    /// Each flight's revisions come back oldest first, though inserted newest first and
    /// interleaved; a flight not asked for is absent.
    #[sqlx::test]
    async fn plans_group_each_flights_revisions_oldest_first(pool: PgPool) {
        for (sid, min, route) in [
            (10_i64, 30, "D"),
            (5, 20, "B"),
            (15, 15, "X"),
            (10, 10, "C"),
            (5, 0, "A"),
        ] {
            sqlx::query(
                "insert into stats.flight_plan (session_id, effective_from, route, cruise_alt) \
                 values ($1, $2, $3, 35000)",
            )
            .bind(sid)
            .bind(t0() + Duration::minutes(min))
            .bind(route)
            .execute(&pool)
            .await
            .unwrap();
        }
        let map = plans(&pool, &[5, 10]).await.unwrap();
        let routes = |sid| {
            map[&sid]
                .iter()
                .map(|p| p.route.clone().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(routes(5), vec!["A", "B"]);
        assert_eq!(routes(10), vec!["C", "D"]);
        assert_eq!(map[&5][1].effective_from, t0() + Duration::minutes(20));
        assert_eq!(map[&5][0].cruise_alt, Some(35_000));
        assert!(!map.contains_key(&15));
    }

    /// The revision in force is the newest at or before the instant.
    #[test]
    fn plan_at_takes_the_newest_revision_at_or_before_the_instant() {
        let rev = |min: i64, route: &str| HistPlan {
            effective_from: t0() + Duration::minutes(min),
            route: Some(route.into()),
            cruise_alt: None,
        };
        let revs = [rev(0, "A"), rev(20, "B")];
        let route = |at: DateTime<Utc>| plan_at(&revs, at).and_then(|p| p.route.clone());
        assert_eq!(route(t0() - Duration::seconds(1)), None);
        assert_eq!(route(t0()).as_deref(), Some("A"));
        assert_eq!(route(t0() + Duration::minutes(19)).as_deref(), Some("A"));
        assert_eq!(route(t0() + Duration::minutes(20)).as_deref(), Some("B"));
        assert_eq!(route(t0() + Duration::hours(5)).as_deref(), Some("B"));
        assert!(plan_at(&[], t0()).is_none());
    }
}
