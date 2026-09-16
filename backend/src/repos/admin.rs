//! Read-only aggregates for the Admin page's landing summary.

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{DailyCount, DailySeries},
};

/// Days covered by each summary series (today included).
pub const SUMMARY_DAYS: i32 = 30;

/// A table whose `created_at` rows are counted per day. A closed set, so the table name
/// interpolated into the query is never caller input.
#[derive(Clone, Copy)]
pub enum CountedTable {
    AuditLogs,
    Users,
}

impl CountedTable {
    fn name(self) -> &'static str {
        match self {
            CountedTable::AuditLogs => "access.audit_logs",
            CountedTable::Users => "identity.users",
        }
    }
}

#[derive(sqlx::FromRow)]
struct DayRow {
    day: chrono::NaiveDate,
    count: i64,
}

/// Rows created per UTC day over the last `days` days, zero-filled, oldest first.
pub async fn daily_counts(
    pool: &PgPool,
    table: CountedTable,
    days: i32,
) -> Result<DailySeries, ApiError> {
    let rows = sqlx::query_as::<_, DayRow>(&format!(
        "with bounds as (select (now() at time zone 'utc')::date as today) \
         select d::date as day, count(t.created_at) as count \
         from bounds, generate_series(bounds.today - ($1 - 1), bounds.today, interval '1 day') as d \
         left join {table} t \
           on t.created_at >= d and t.created_at < d + interval '1 day' \
         group by d order by d",
        table = table.name(),
    ))
    .bind(days)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    let points: Vec<DailyCount> = rows
        .into_iter()
        .map(|r| DailyCount {
            day: r.day,
            count: r.count,
        })
        .collect();
    Ok(DailySeries {
        total: points.iter().map(|p| p.count).sum(),
        points,
    })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use sqlx::PgPool;

    use super::*;

    async fn audit_at(pool: &PgPool, days_ago: i64) {
        sqlx::query(
            "insert into access.audit_logs (action, resource_type, created_at) values ('test.create', 'test', $1)",
        )
        .bind(Utc::now() - Duration::days(days_ago))
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn daily_counts_zero_fill_the_window_oldest_first(pool: PgPool) {
        audit_at(&pool, 0).await;
        audit_at(&pool, 0).await;
        audit_at(&pool, 2).await;
        audit_at(&pool, 45).await; // outside the window

        let series = daily_counts(&pool, CountedTable::AuditLogs, 30)
            .await
            .unwrap();

        assert_eq!(series.points.len(), 30);
        assert!(series.points.windows(2).all(|w| w[0].day < w[1].day));
        assert_eq!(series.points.last().unwrap().day, Utc::now().date_naive());
        assert_eq!(series.points[29].count, 2);
        assert_eq!(series.points[27].count, 1);
        assert_eq!(series.total, 3);
    }

    #[sqlx::test]
    async fn daily_counts_empty_table_is_all_zero(pool: PgPool) {
        let series = daily_counts(&pool, CountedTable::Users, 7).await.unwrap();
        assert_eq!(series.points.len(), 7);
        assert_eq!(series.total, 0);
    }
}
