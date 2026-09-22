//! The single database round trip behind `GET /metrics` (#382).
//!
//! Everything else the exporter reports is already in memory; these six numbers are not. They are
//! fetched as one query of scalar subqueries rather than six calls, because a scrape runs on
//! Prometheus' interval and should cost one round trip, not six.
//!
//! **The "active" predicates below are mirrored from [`crate::repos::public`]**, whose module doc
//! is the canonical definition of active for the public advisories board: restrictions and ground
//! stops are published and not past their end, GDPs are published, programs are live (not past
//! `active_until`), FCAs are enabled. If that definition changes there, it must change here — the
//! two are deliberately identical so a dashboard and the public board can never disagree about how
//! many TMIs are running.

use sqlx::PgPool;

use crate::{errors::ApiError, metrics::DomainCounts};

pub async fn domain_counts(pool: &PgPool) -> Result<DomainCounts, ApiError> {
    sqlx::query_as::<_, DomainCounts>(
        "select \
           (select count(*) from tmu.tmis \
             where status = 'published' and (stop_time is null or stop_time > now())) \
             as active_tmis, \
           (select count(*) from tmu.ground_stops \
             where status = 'published' \
               and (tmu.ground_stop_until_ts(created_at, until) is null \
                    or tmu.ground_stop_until_ts(created_at, until) > now())) \
             as active_ground_stops, \
           (select count(*) from tmu.gdp where status = 'published') as active_gdps, \
           (select count(*) from tmu.programs \
             where active_until is null or active_until > now()) as active_programs, \
           (select count(*) from flow.fca where enabled) as enabled_fcas, \
           (select coalesce(sum(s.delay_min), 0)::bigint from tmu.gdp_slot s \
              join tmu.gdp g on g.id = s.gdp_id \
             where g.status = 'published') as gdp_delay_minutes",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| {
        // The handler degrades to the in-memory series when this fails, so without this line a
        // permanently-wrong query would show up only as six missing gauges and no explanation.
        tracing::warn!(error = %e, "metrics domain aggregate failed");
        ApiError::Internal
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty database must answer all-zero rather than erroring — the query has to be valid
    /// against the real schema, which is the half of this a unit test cannot check. A scrape on a
    /// quiet deployment is the common case, so this is also the common path.
    #[sqlx::test]
    async fn an_empty_database_reports_nothing_active(pool: PgPool) {
        let counts = domain_counts(&pool).await.expect("query is valid");
        assert_eq!(counts.active_tmis, 0);
        assert_eq!(counts.active_ground_stops, 0);
        assert_eq!(counts.active_gdps, 0);
        assert_eq!(counts.active_programs, 0);
        assert_eq!(counts.enabled_fcas, 0);
        assert_eq!(counts.gdp_delay_minutes, 0);
    }

    /// The counts must follow the *same* notion of "active" the public board uses, not merely
    /// "row exists". Each insert below is paired with one that should not be counted.
    #[sqlx::test]
    async fn only_rows_the_public_board_calls_active_are_counted(pool: PgPool) {
        sqlx::query(
            "insert into tmu.tmis (id, requesting, providing, restriction, status, stop_time) values \
               ('t-live',  'ZDC', 'ZNY', '20 MIT', 'published', now() + interval '1 hour'), \
               ('t-open',  'ZDC', 'ZNY', '20 MIT', 'published', null), \
               ('t-past',  'ZDC', 'ZNY', '20 MIT', 'published', now() - interval '1 hour'), \
               ('t-draft', 'ZDC', 'ZNY', '20 MIT', 'draft',     null)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "insert into tmu.gdp (id, airport, aar, start_time, end_time, status) values \
               ('g-pub',   'KDCA', 30, '1200', '1400', 'published'), \
               ('g-draft', 'KJFK', 30, '1200', '1400', 'draft')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "insert into tmu.gdp_slot (gdp_id, callsign, original_eta, cta, delay_min) values \
               ('g-pub',   'AAL1', now(), now(), 12), \
               ('g-pub',   'AAL2', now(), now(), 8), \
               ('g-draft', 'AAL3', now(), now(), 99)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let counts = domain_counts(&pool).await.expect("query is valid");
        // 't-live' and 't-open'; not the expired one, not the draft.
        assert_eq!(counts.active_tmis, 2);
        // Only the published GDP.
        assert_eq!(counts.active_gdps, 1);
        // 12 + 8 — the draft GDP's 99 minutes are not delay anyone is serving.
        assert_eq!(counts.gdp_delay_minutes, 20);
    }
}
