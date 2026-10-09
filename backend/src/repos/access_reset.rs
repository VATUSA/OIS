//! The access reset's runs (#806), and the advisory locks that keep a reset and the VATUSA division
//! pull from running at once on any backend replica.

use chrono::{DateTime, Utc};
use sqlx::{Connection, PgConnection, PgPool};

use crate::errors::ApiError;
use crate::models::{AccessResetBody, AccessResetFailure, AccessResetRun};

/// The first key of every OIS advisory lock (the two-key form): "OIS" in ASCII.
const LOCK_CLASS: i32 = 0x004F_4953;
/// Held by the division pull, and by a reset from before its pull until its last member.
const DIVISION_LOCK: i32 = 1;
/// Held by a reset for its whole run, so only one runs at a time.
const RESET_LOCK: i32 = 2;

/// What a run that a dead process left `running` reports: its members' audit entries are the only
/// record of how far it got.
const INTERRUPTED: &str = "reset_interrupted";
const INTERRUPTED_MESSAGE: &str = "the backend running this reset stopped before it finished; the \
                                   members already reset stay reset, the rest are untouched — run it \
                                   again to finish";

/// Session advisory locks, held on a connection taken out of the pool. Dropping it closes that
/// connection, and Postgres releases every lock it held, whether the run finished, panicked or was
/// cancelled. A pooled connection would go back to the pool still holding them.
pub struct AdvisoryLocks(PgConnection);

impl AdvisoryLocks {
    async fn connect(pool: &PgPool) -> Result<Self, sqlx::Error> {
        Ok(Self(pool.acquire().await?.detach()))
    }

    /// Release every lock now and close the connection. Postgres would release them anyway once it
    /// noticed the dropped connection, but only some time later; a reset that starts right after a run
    /// finishes must find the reset lock free.
    pub async fn release(mut self) {
        if let Err(e) = sqlx::query("select pg_advisory_unlock_all()")
            .execute(&mut self.0)
            .await
        {
            tracing::warn!(error = %e, "could not release the access reset's advisory locks; closing their connection");
        }
        let _ = self.0.close().await;
    }

    /// Wait for the division lock.
    pub async fn lock_division(&mut self) -> Result<(), sqlx::Error> {
        sqlx::query("select pg_advisory_lock($1, $2)")
            .bind(LOCK_CLASS)
            .bind(DIVISION_LOCK)
            .execute(&mut self.0)
            .await
            .map(|_| ())
    }
}

/// Wait for the division lock, on a connection of its own. The division pull holds it while it runs.
pub async fn lock_division(pool: &PgPool) -> Result<AdvisoryLocks, sqlx::Error> {
    let mut locks = AdvisoryLocks::connect(pool).await?;
    locks.lock_division().await?;
    Ok(locks)
}

/// Claim the reset lock, or `None` when another reset, on any replica, holds it.
///
/// The connection leaves the pool before the attempt, not after it succeeds. The attempt runs in the
/// request, which is dropped when the client goes away. Dropped after Postgres granted the lock but
/// before the reply was read, a pooled connection would go back to the pool still holding the lock,
/// and keep it for as long as the pool keeps the connection. A detached one is closed instead. That
/// window is one round trip and no test can land in it reliably, so this order is the guard.
pub async fn try_lock_reset(pool: &PgPool) -> Result<Option<AdvisoryLocks>, sqlx::Error> {
    let mut locks = AdvisoryLocks::connect(pool).await?;
    let claimed: bool = sqlx::query_scalar("select pg_try_advisory_lock($1, $2)")
        .bind(LOCK_CLASS)
        .bind(RESET_LOCK)
        .fetch_one(&mut locks.0)
        .await?;
    Ok(claimed.then_some(locks))
}

/// The Postgres backend pid holding the reset lock, if any, for the log when a start gives up: an
/// operator can end a stuck holder with `pg_terminate_backend`.
pub async fn reset_lock_holder(pool: &PgPool) -> Result<Option<i32>, ApiError> {
    sqlx::query_scalar(
        "select pid from pg_locks where locktype = 'advisory' and granted and objsubid = 2 \
         and database = (select oid from pg_database where datname = current_database()) \
         and classid = $1::int8::oid and objid = $2::int8::oid limit 1",
    )
    .bind(i64::from(LOCK_CLASS))
    .bind(i64::from(RESET_LOCK))
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

// The two SQL fragments below are spliced into queries with `format!`. Both are constants of this
// module, never input; every value they compare against is bound.

/// `true` while some session in this database holds the reset lock.
const RESET_LOCK_HELD: &str = "exists (select 1 from pg_locks \
     where locktype = 'advisory' and granted and objsubid = 2 \
       and database = (select oid from pg_database where datname = current_database()) \
       and classid = $1::int8::oid and objid = $2::int8::oid)";

/// How many members a run `r` still `running` has reset so far, counted from their audit entries
/// (0 for a finished run, which stored its own count). Runs never overlap, so every reset entry since
/// it started is its own.
const RESET_SO_FAR: &str = "(case when r.status = 'running' then (select count(*) from access.audit_logs a \
     where a.resource_type = 'USER_ACCESS' and a.reason like 'Reset to VATUSA: %' \
       and a.created_at >= r.started_at) else 0 end)";

/// Record a new run as `running`, and return its id. The caller holds the reset lock, so any other
/// row still `running` belongs to a process that died: it is closed as interrupted first.
pub async fn start_run(pool: &PgPool, started_by: &str, reason: &str) -> Result<String, ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query(&format!(
        "update access.vatusa_reset_runs r set status = 'failed', finished_at = now(), \
         failure = jsonb_build_object('error', $1::text, 'message', $2::text, 'users_reset', \
         {RESET_SO_FAR}) where status = 'running'"
    ))
    .bind(INTERRUPTED)
    .bind(INTERRUPTED_MESSAGE)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    let id = sqlx::query_scalar(
        "insert into access.vatusa_reset_runs (started_by, reason) values ($1, $2) returning id",
    )
    .bind(started_by)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(id)
}

/// Store a run's outcome.
pub async fn finish_run(
    pool: &PgPool,
    id: &str,
    outcome: Result<&AccessResetBody, &AccessResetFailure>,
) -> Result<(), ApiError> {
    let (status, result, failure) = match outcome {
        Ok(body) => ("succeeded", Some(serde_json::to_string(body)), None),
        Err(failure) => ("failed", None, Some(serde_json::to_string(failure))),
    };
    let stored = |json: Option<serde_json::Result<String>>| match json {
        Some(Err(e)) => {
            tracing::error!(error = %e, run_id = id, "an access reset's outcome could not be serialized");
            None
        }
        other => other.and_then(Result::ok),
    };
    let (result, failure) = (stored(result), stored(failure));
    // Serialized to text and cast, as the audit log's snapshots are: sqlx is built without `json`.
    sqlx::query(
        "update access.vatusa_reset_runs set status = $2, finished_at = now(), \
         result = $3::jsonb, failure = $4::jsonb where id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(result)
    .bind(failure)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|_| ApiError::Internal)
}

/// The run in progress, if any: the newest `running` row.
pub async fn running_run(pool: &PgPool) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar(
        "select id from access.vatusa_reset_runs where status = 'running' \
         order by started_at desc limit 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// One run, or `None` if there is no such run. A row still `running` while no session holds the reset
/// lock was left by a process that died, and is reported as failed with how far it got.
pub async fn fetch_run(pool: &PgPool, id: &str) -> Result<Option<AccessResetRun>, ApiError> {
    type Row = (
        String,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
        bool,
        i64,
    );
    let row: Option<Row> = sqlx::query_as(&format!(
        "select r.status, r.started_at, r.finished_at, r.result::text, r.failure::text, \
         {RESET_LOCK_HELD}, {RESET_SO_FAR} from access.vatusa_reset_runs r where r.id = $3"
    ))
    .bind(i64::from(LOCK_CLASS))
    .bind(i64::from(RESET_LOCK))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let Some((status, started_at, finished_at, result, failure, live, reset_so_far)) = row else {
        return Ok(None);
    };
    let mut run = AccessResetRun {
        id: id.to_string(),
        status,
        started_at,
        finished_at,
        result: stored_json(id, result),
        failure: stored_json(id, failure),
    };
    // The row comes from the statement's snapshot and the locks are read live, so a run that stored
    // its result and released its locks while the statement ran looks `running` with no holder. Read
    // the status again before calling it interrupted.
    if run.status == "running" && !live && still_running(pool, id).await? {
        run.status = "failed".to_string();
        run.failure = Some(AccessResetFailure {
            error: INTERRUPTED.to_string(),
            message: INTERRUPTED_MESSAGE.to_string(),
            users_reset: reset_so_far,
        });
    }
    Ok(Some(run))
}

async fn still_running(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    sqlx::query_scalar("select status = 'running' from access.vatusa_reset_runs where id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(|running| running.unwrap_or(false))
        .map_err(|_| ApiError::Internal)
}

/// A stored result or failure, read back. One that no longer parses is logged rather than dropped
/// silently.
fn stored_json<T: serde::de::DeserializeOwned>(id: &str, json: Option<String>) -> Option<T> {
    match serde_json::from_str(json.as_deref()?) {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::error!(error = %e, run_id = id, "an access reset's stored outcome does not parse");
            None
        }
    }
}
