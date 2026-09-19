//! Manually excluded ("bogus") flights — see migration 0079 and issue #342. Writes are
//! facility-scoped in the handler; the repo is unscoped, mirroring `airport_configs` /
//! `airport_surface`.
//!
//! Every read filters on `expires_at > now()`, so an expired exclusion simply stops applying — there
//! is no reaper job. Rows are cleaned up opportunistically by [`clear_departed`] when the callsign
//! leaves the VATSIM feed.

use std::collections::{HashMap, HashSet};

use sqlx::PgPool;

use crate::{errors::ApiError, models::FlightExclusionBody};

const EXCLUSION_SELECT: &str = "select e.id, e.callsign, e.artcc, e.reason, e.created_at, \
     e.created_by, e.expires_at, u.display_name as created_by_name \
     from flow.manual_flight_exclusion e \
     left join identity.users u on u.id = e.created_by";

/// The live exclusions for one ARTCC, newest first.
pub async fn list_by_artcc(
    pool: &PgPool,
    artcc: &str,
) -> Result<Vec<FlightExclusionBody>, ApiError> {
    sqlx::query_as::<_, FlightExclusionBody>(&format!(
        "{EXCLUSION_SELECT} where e.artcc = $1 and e.expires_at > now() order by e.created_at desc"
    ))
    .bind(artcc)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Every live exclusion, grouped by ARTCC — for `AppState::flight_exclusions`
/// (`jobs::spawn_flight_exclusions_refresh` and the force-reload on write in
/// `handlers::flight_exclusions`), so the DB-less feed/flow surfaces can filter a bogus callsign
/// without querying inline.
pub async fn load_all(pool: &PgPool) -> Result<HashMap<String, HashSet<String>>, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "select artcc, callsign from flow.manual_flight_exclusion where expires_at > now()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let mut by_artcc: HashMap<String, HashSet<String>> = HashMap::new();
    for (artcc, callsign) in rows {
        by_artcc.entry(artcc).or_default().insert(callsign);
    }
    Ok(by_artcc)
}

/// Exclude a callsign for an ARTCC until `expires_at`. Re-removing an already-excluded flight
/// refreshes the note, the actor and the TTL rather than erroring.
pub async fn upsert(
    pool: &PgPool,
    artcc: &str,
    callsign: &str,
    reason: &str,
    ttl_hours: i64,
    actor: &str,
) -> Result<FlightExclusionBody, ApiError> {
    let id: String = sqlx::query_scalar(
        "insert into flow.manual_flight_exclusion (callsign, artcc, reason, created_by, expires_at) \
         values ($1, $2, $3, $4, now() + make_interval(hours => $5::int)) \
         on conflict (artcc, callsign) do update set \
             reason = excluded.reason, \
             created_by = excluded.created_by, \
             created_at = now(), \
             expires_at = excluded.expires_at \
         returning id",
    )
    .bind(callsign)
    .bind(artcc)
    .bind(reason)
    .bind(actor)
    .bind(ttl_hours)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    get(pool, &id).await?.ok_or(ApiError::Internal)
}

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<FlightExclusionBody>, ApiError> {
    sqlx::query_as::<_, FlightExclusionBody>(&format!("{EXCLUSION_SELECT} where e.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Restore a flight — the undo path. Returns false when nothing was excluded.
pub async fn delete(pool: &PgPool, artcc: &str, callsign: &str) -> Result<bool, ApiError> {
    let r =
        sqlx::query("delete from flow.manual_flight_exclusion where artcc = $1 and callsign = $2")
            .bind(artcc)
            .bind(callsign)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// Drop exclusions whose callsign is no longer in the VATSIM feed — the primary auto-clear (#342):
/// a corrected or returning flight must not stay hidden. `live` is every callsign in the current
/// snapshot. A snapshot with no callsigns at all is ignored, so a feed outage can't wipe every
/// exclusion at once.
pub async fn clear_departed(pool: &PgPool, live: &[String]) -> Result<u64, ApiError> {
    if live.is_empty() {
        return Ok(0);
    }
    let r = sqlx::query("delete from flow.manual_flight_exclusion where callsign <> all($1)")
        .bind(live)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope_test_support::seed_user;

    /// Removing the same bogus flight twice refreshes the row rather than erroring — a controller
    /// re-clicking ✕ must not get a 500 off the unique index.
    #[sqlx::test]
    async fn excluding_the_same_callsign_twice_refreshes_it(pool: sqlx::PgPool) {
        let user = seed_user(&pool).await;
        let first = upsert(&pool, "ZDC", "BOGUS1", "teleporting", 2, &user)
            .await
            .unwrap();
        let again = upsert(&pool, "ZDC", "BOGUS1", "still bad", 2, &user)
            .await
            .unwrap();
        assert_eq!(first.id, again.id, "the row is reused, not duplicated");
        assert_eq!(again.reason, "still bad");
        assert_eq!(list_by_artcc(&pool, "ZDC").await.unwrap().len(), 1);
    }

    /// Exclusions are per-facility: one ARTCC's removal is invisible to another's list and to the
    /// cache bucket another ARTCC reads.
    #[sqlx::test]
    async fn exclusions_are_scoped_to_their_artcc(pool: sqlx::PgPool) {
        let user = seed_user(&pool).await;
        upsert(&pool, "ZDC", "BOGUS1", "", 2, &user).await.unwrap();
        upsert(&pool, "ZNY", "BOGUS2", "", 2, &user).await.unwrap();

        let zdc = list_by_artcc(&pool, "ZDC").await.unwrap();
        assert_eq!(zdc.len(), 1);
        assert_eq!(zdc[0].callsign, "BOGUS1");

        let all = load_all(&pool).await.unwrap();
        assert!(all["ZDC"].contains("BOGUS1"));
        assert!(all["ZNY"].contains("BOGUS2"));
        assert!(!all["ZDC"].contains("BOGUS2"));
    }

    /// The TTL backstop (#342): readers filter on `expires_at`, so an elapsed exclusion stops
    /// applying without any reaper. A zero-hour TTL is already expired.
    #[sqlx::test]
    async fn an_expired_exclusion_stops_applying(pool: sqlx::PgPool) {
        let user = seed_user(&pool).await;
        upsert(&pool, "ZDC", "STALE1", "", 0, &user).await.unwrap();

        assert!(
            list_by_artcc(&pool, "ZDC").await.unwrap().is_empty(),
            "an expired exclusion must not be listed"
        );
        assert!(
            !load_all(&pool)
                .await
                .unwrap()
                .get("ZDC")
                .is_some_and(|s| s.contains("STALE1")),
            "an expired exclusion must not reach the filtering cache"
        );
    }

    /// The primary auto-clear: once the callsign leaves the feed the exclusion is dropped, so a
    /// corrected or returning flight is never hidden forever.
    #[sqlx::test]
    async fn a_callsign_that_left_the_feed_is_cleared(pool: sqlx::PgPool) {
        let user = seed_user(&pool).await;
        upsert(&pool, "ZDC", "GONE1", "", 2, &user).await.unwrap();
        upsert(&pool, "ZDC", "STILLHERE", "", 2, &user)
            .await
            .unwrap();

        let cleared = clear_departed(&pool, &["STILLHERE".to_string()])
            .await
            .unwrap();
        assert_eq!(cleared, 1);
        let left = list_by_artcc(&pool, "ZDC").await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].callsign, "STILLHERE");
    }

    /// An empty snapshot (feed outage / not yet loaded) must not wipe every exclusion.
    #[sqlx::test]
    async fn an_empty_feed_snapshot_clears_nothing(pool: sqlx::PgPool) {
        let user = seed_user(&pool).await;
        upsert(&pool, "ZDC", "BOGUS1", "", 2, &user).await.unwrap();

        assert_eq!(clear_departed(&pool, &[]).await.unwrap(), 0);
        assert_eq!(list_by_artcc(&pool, "ZDC").await.unwrap().len(), 1);
    }

    /// Restore is the undo path, and it is scoped: another facility can't clear your removal.
    #[sqlx::test]
    async fn restoring_is_scoped_and_reports_a_miss(pool: sqlx::PgPool) {
        let user = seed_user(&pool).await;
        upsert(&pool, "ZDC", "BOGUS1", "", 2, &user).await.unwrap();

        assert!(
            !delete(&pool, "ZNY", "BOGUS1").await.unwrap(),
            "another facility must not clear ZDC's removal"
        );
        assert!(!delete(&pool, "ZDC", "NOSUCH").await.unwrap());
        assert!(delete(&pool, "ZDC", "BOGUS1").await.unwrap());
        assert!(list_by_artcc(&pool, "ZDC").await.unwrap().is_empty());
    }
}
