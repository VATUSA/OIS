//! Request volume per credential per hour (#611, migration 0121). `add` is the flush job's write;
//! `prune` keeps 7 days. The key and service-account list queries read it.

use sqlx::PgPool;

use crate::{errors::ApiError, rate_limit::CredentialUsage};

/// Add `counts` to the current hour's rows — additive, so every replica's flush sums into one row.
pub async fn add(pool: &PgPool, counts: &[CredentialUsage]) -> Result<(), ApiError> {
    for c in counts {
        sqlx::query(
            "insert into access.credential_usage (kind, credential_id, hour, requests, refused) \
             values ($1, $2, date_trunc('hour', now()), $3, $4) \
             on conflict (kind, credential_id, hour) do update \
             set requests = access.credential_usage.requests + excluded.requests, \
                 refused = access.credential_usage.refused + excluded.refused",
        )
        .bind(c.kind)
        .bind(&c.credential_id)
        .bind(i64::try_from(c.usage.requests).unwrap_or(i64::MAX))
        .bind(i64::try_from(c.usage.refused).unwrap_or(i64::MAX))
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    Ok(())
}

/// Drop hours older than the 7 days the UI could show.
pub async fn prune(pool: &PgPool) -> Result<u64, ApiError> {
    sqlx::query("delete from access.credential_usage where hour < now() - interval '7 days'")
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
        .map_err(|_| ApiError::Internal)
}
