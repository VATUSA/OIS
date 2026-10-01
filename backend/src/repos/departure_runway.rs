//! Which runway a departure will use (#509, sub-issue A of #434). See migration `0089`.
//!
//! # Why `(icao, callsign)` and not `(fca_id, callsign)`
//!
//! A runway belongs to the **physical departure**, not to any initiative metering it. The two
//! existing per-flight tables key on the initiative — `flow.fca_release` on `(fca_id, callsign)`,
//! `tmu.gdp_slot` on `(gdp_id, callsign)` — which is right for a frozen CTA or a slot, because those
//! facts *are* the initiative's. `IdstFlight` is one row per (FCA, departure), so a flight metered by
//! two FCAs appears twice; keying a runway that way would let one aircraft hold two different
//! runways with nothing to reconcile them.
//!
//! What that key settles for the rest of #434:
//!
//! * **Two FCAs, one assignment.** A flight metered twice has a single runway, shared. The primary
//!   key makes the alternative unrepresentable rather than merely discouraged.
//! * **A cross-FCA swap (#514) does not touch this table.** Swapping two flights' release times
//!   moves `cta_ms`/`edct_ms` between aircraft; each runway stays with its own aircraft, so there is
//!   nothing to exchange here.
//! * **Nothing cascades.** This is the first callsign-keyed table with no parent row, so
//!   [`crate::jobs`]'s prune pass is what removes stale assignments.
//!
//! # No HTTP endpoint yet
//!
//! Deliberate. The writers are #511's prediction ladder and #512's facility rules; a route with no
//! caller would be scope #509 does not ask for, and would have to be designed before the thing that
//! uses it exists. The row type lives here rather than in `models` for the same reason — it is not
//! part of the API surface, and `repos::audit::AuditEntry` is the precedent for a repo-local struct.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;

/// Batch size for one prune pass, mirroring `repos::audit`'s batched delete so a sweep cannot hold a
/// long lock on the table.
const PRUNE_BATCH: i64 = 5_000;

/// The precedence ladder, ascending, as a SQL array literal.
///
/// `array_position` over this gives each source its rank, which is what
/// [`assign`]'s `on conflict` guard compares. Written once here rather than as a `case` expression
/// duplicated for both sides of that comparison. It is a static literal — no user input reaches it.
const SOURCE_LADDER: &str = "array['auto', 'config', 'rule', 'manual']";

/// Which rung of #511's ladder produced an assignment.
///
/// An enum rather than a bare `&str` so a caller cannot invent a source the `check` constraint in
/// `0089` would reject at runtime. The ordering between them is **not** on this type: it lives in
/// the SQL guard, because a read-modify-write in Rust would race two concurrent derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunwaySource {
    /// A controller set this runway by hand. Outranks everything.
    Manual,
    /// A facility's configured gate/SID rule matched (#512).
    Rule,
    /// The active airport config's `departure_runways` (#509's column).
    Config,
    /// Picked automatically with no rule or config behind it.
    Auto,
}

impl RunwaySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Rule => "rule",
            Self::Config => "config",
            Self::Auto => "auto",
        }
    }
}

/// One departure's assigned runway.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DepartureRunwayAssignment {
    pub icao: String,
    pub callsign: String,
    pub runway: String,
    pub source: String,
    pub updated_at: DateTime<Utc>,
}

/// Record a runway for `(icao, callsign)`, unless something higher up the ladder already set it.
///
/// Returns whether the row was written. `false` means an existing assignment outranked this one —
/// which is the case that protects a controller's manual override from being overwritten by the next
/// derive, and the reason the comparison is in the `on conflict` clause rather than in a caller.
/// Nothing can bypass it by forgetting to check first.
///
/// A write of *equal* rank replaces, so a controller can change their own override and a re-derive
/// can correct a stale `config` pick.
pub async fn assign(
    pool: &PgPool,
    icao: &str,
    callsign: &str,
    runway: &str,
    source: RunwaySource,
    actor: Option<&str>,
) -> Result<bool, ApiError> {
    let sql = format!(
        "insert into flow.departure_runway_assignment \
             (icao, callsign, runway, source, updated_by) \
         values ($1, $2, $3, $4, $5) \
         on conflict (icao, callsign) do update set \
             runway = excluded.runway, \
             source = excluded.source, \
             updated_by = excluded.updated_by, \
             updated_at = now() \
         where array_position({SOURCE_LADDER}, excluded.source) \
            >= array_position({SOURCE_LADDER}, flow.departure_runway_assignment.source)"
    );
    sqlx::query(&sql)
        .bind(icao)
        .bind(callsign)
        .bind(runway)
        .bind(source.as_str())
        .bind(actor)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() > 0)
        .map_err(|_| ApiError::Internal)
}

pub async fn get(
    pool: &PgPool,
    icao: &str,
    callsign: &str,
) -> Result<Option<DepartureRunwayAssignment>, ApiError> {
    sqlx::query_as::<_, DepartureRunwayAssignment>(
        "select icao, callsign, runway, source, updated_at \
         from flow.departure_runway_assignment where icao = $1 and callsign = $2",
    )
    .bind(icao)
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn list_for_airport(
    pool: &PgPool,
    icao: &str,
) -> Result<Vec<DepartureRunwayAssignment>, ApiError> {
    sqlx::query_as::<_, DepartureRunwayAssignment>(
        "select icao, callsign, runway, source, updated_at \
         from flow.departure_runway_assignment where icao = $1 order by callsign",
    )
    .bind(icao)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Delete assignments last touched before `before`. Batched like `repos::audit::prune_audit_logs`.
pub async fn prune(pool: &PgPool, before: DateTime<Utc>) -> Result<u64, ApiError> {
    sqlx::query(
        "delete from flow.departure_runway_assignment \
         where ctid in ( \
             select ctid from flow.departure_runway_assignment where updated_at < $1 limit $2 \
         )",
    )
    .bind(before)
    .bind(PRUNE_BATCH)
    .execute(pool)
    .await
    .map(|r| r.rows_affected())
    .map_err(|_| ApiError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_user(pool: &PgPool) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into identity.users (full_name, display_name) \
             values ('Test User', 'Test User') returning id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// #509 AC 4. The acceptance criterion says "survives a feed recompute", but the feed subsystem
    /// holds no DB handle (`AGENTS.md`), so no feed poll can reach this table at all — that half is
    /// true by construction. What needs defending is the rung ordering: a controller sets a runway by
    /// hand, the next derive runs, and the override has to still be there.
    #[sqlx::test]
    async fn a_derived_assignment_does_not_overwrite_a_manual_one(pool: PgPool) {
        assert!(
            assign(&pool, "KJFK", "AAL100", "13R", RunwaySource::Manual, None)
                .await
                .unwrap()
        );

        for lower in [RunwaySource::Rule, RunwaySource::Config, RunwaySource::Auto] {
            let wrote = assign(&pool, "KJFK", "AAL100", "22L", lower, None)
                .await
                .unwrap();
            assert!(!wrote, "{lower:?} must not outrank a manual override");
        }

        let got = get(&pool, "KJFK", "AAL100").await.unwrap().unwrap();
        assert_eq!(got.runway, "13R");
        assert_eq!(got.source, "manual");
    }

    /// The ladder is a precedence order, not a lock: a rule outranks a config pick, and a config pick
    /// outranks a bare auto one. Asserted in both directions so an inverted comparison fails.
    #[sqlx::test]
    async fn a_higher_rung_replaces_a_lower_one_but_not_the_reverse(pool: PgPool) {
        assert!(
            assign(&pool, "KJFK", "UAL1", "04L", RunwaySource::Auto, None)
                .await
                .unwrap()
        );
        assert!(
            assign(&pool, "KJFK", "UAL1", "13R", RunwaySource::Config, None)
                .await
                .unwrap(),
            "config outranks auto"
        );
        assert!(
            assign(&pool, "KJFK", "UAL1", "22R", RunwaySource::Rule, None)
                .await
                .unwrap(),
            "rule outranks config"
        );
        assert!(
            !assign(&pool, "KJFK", "UAL1", "31L", RunwaySource::Config, None)
                .await
                .unwrap(),
            "config must not claw back from rule"
        );
        assert_eq!(
            get(&pool, "KJFK", "UAL1").await.unwrap().unwrap().runway,
            "22R"
        );
    }

    /// A write of equal rank replaces, so a controller can correct their own override and a re-derive
    /// can fix a stale pick. Without this the first assignment of any rung would be permanent.
    #[sqlx::test]
    async fn an_equal_rung_replaces(pool: PgPool) {
        assign(&pool, "KJFK", "DAL2", "13R", RunwaySource::Manual, None)
            .await
            .unwrap();
        assert!(
            assign(&pool, "KJFK", "DAL2", "22L", RunwaySource::Manual, None)
                .await
                .unwrap()
        );
        assert_eq!(
            get(&pool, "KJFK", "DAL2").await.unwrap().unwrap().runway,
            "22L"
        );
    }

    /// The two-FCA decision, asserted rather than only documented. A flight metered by two FCAs is
    /// two `IdstFlight` rows, so a ladder running per-FCA assigns twice for one aircraft. Keyed on
    /// `(icao, callsign)` that converges to one row; keyed on `(fca_id, callsign)` it would have been
    /// two rows free to disagree.
    #[sqlx::test]
    async fn a_flight_metered_by_two_fcas_has_one_assignment(pool: PgPool) {
        for _ in 0..2 {
            assign(&pool, "KJFK", "SWA3", "13R", RunwaySource::Rule, None)
                .await
                .unwrap();
        }
        let all = list_for_airport(&pool, "KJFK").await.unwrap();
        assert_eq!(all.len(), 1, "one physical departure, one runway: {all:?}");
        assert_eq!(all[0].callsign, "SWA3");
    }

    /// Nothing cascades into this table, so the prune is the only thing that removes a row.
    #[sqlx::test]
    async fn prune_drops_stale_rows_and_keeps_fresh_ones(pool: PgPool) {
        assign(&pool, "KJFK", "OLD1", "13R", RunwaySource::Auto, None)
            .await
            .unwrap();
        assign(&pool, "KJFK", "NEW1", "13R", RunwaySource::Auto, None)
            .await
            .unwrap();
        sqlx::query(
            "update flow.departure_runway_assignment set updated_at = now() - interval '2 days' \
             where callsign = 'OLD1'",
        )
        .execute(&pool)
        .await
        .unwrap();

        let deleted = prune(&pool, Utc::now() - chrono::Duration::hours(12))
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        assert!(get(&pool, "KJFK", "OLD1").await.unwrap().is_none());
        assert!(get(&pool, "KJFK", "NEW1").await.unwrap().is_some());
    }

    /// `updated_by` is a real FK, so an actor id has to be storable — the ladder's manual rung will
    /// always have one.
    #[sqlx::test]
    async fn an_assignment_records_who_set_it(pool: PgPool) {
        let user = seed_user(&pool).await;
        assign(
            &pool,
            "KJFK",
            "JBU4",
            "13R",
            RunwaySource::Manual,
            Some(&user),
        )
        .await
        .unwrap();

        let stored: Option<String> = sqlx::query_scalar(
            "select updated_by from flow.departure_runway_assignment where callsign = 'JBU4'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored.as_deref(), Some(user.as_str()));
    }
}
