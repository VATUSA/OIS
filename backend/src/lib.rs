pub mod advisory;
#[cfg(test)]
mod alloc_probe;
pub mod audit;
pub mod auth;
pub mod config;
pub mod deprecation;
#[cfg(test)]
mod docs_tests;
pub mod errors;
pub mod feed;
pub mod handlers;
pub mod job_registry;
pub mod jobs;
pub mod metrics;
pub mod models;
pub mod openapi;
pub mod rate_limit;
pub mod realtime;
pub mod repos;
pub mod reqlog;
pub mod router;
#[cfg(test)]
pub(crate) mod scope_test_support;
pub mod secrets;
pub mod state;
pub(crate) mod text;
pub mod tmi;

use std::net::SocketAddr;

use tracing_subscriber::{EnvFilter, fmt};

/// Full build version, e.g. "1.0.1-a1b2c3d". Set by build.rs (from the root VERSION file + commit,
/// or the OIS_VERSION env passed by CI); falls back to the crate version if unset.
pub const VERSION: &str = match option_env!("OIS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub async fn run() -> color_eyre::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let state = state::AppState::from_env().await?;
    run_startup_migrations(&state).await?;
    demote_removed_server_admins(&state).await;

    // Drains the Prometheus recorder on a timer (#382). Required even when nothing scrapes:
    // the observability stack is opt-in, and an unscraped recorder retains every latency sample.
    metrics::spawn_upkeep(state.metrics.clone());

    feed::spawn_poller(state.feed.clone(), state.events.clone());
    feed::facilities::spawn_refresh(state.facilities.clone());
    feed::tracon::spawn_refresh(state.tracons.clone());
    // Airport coordinate database: fetched at startup and retried periodically (#216) — a failed
    // boot fetch no longer permanently strands the feed's airport map empty.
    jobs::spawn_airports_refresh(state.jobs.clone(), state.feed.clone());
    jobs::spawn_nav_refresh(
        state.jobs.clone(),
        state.nav.clone(),
        state.nav_refreshed.clone(),
    );
    jobs::spawn_winds_refresh(
        state.jobs.clone(),
        state.feed.clone(),
        state.winds.clone(),
        state.winds_refreshed.clone(),
        state.db.clone(),
    );
    if let Some(pool) = state.db.clone() {
        // Realtime nudges from the other replicas (#649). A failure here only costs cross-replica
        // nudges — clients still poll — so it is logged, not fatal.
        if let Err(e) = state.events.start_listener().await {
            tracing::warn!(error = %e, "realtime: cross-replica listener did not start");
        }
        jobs::spawn_cleanup(state.jobs.clone(), pool.clone());
        // One-time desktop sign-in codes expire in 60s; this removes the dead rows (#346).
        jobs::spawn_desktop_auth_code_prune(state.jobs.clone(), pool.clone());
        jobs::spawn_outbound_job_reaper(state.jobs.clone(), pool.clone());
        jobs::spawn_audit_log_prune(state.jobs.clone(), pool.clone());
        jobs::spawn_diagnostics_report_prune(state.jobs.clone(), pool.clone());
        jobs::spawn_departure_runway_prune(state.jobs.clone(), pool.clone());
        // Predict a departure runway for pending departures (#511). After the gates refresh above, so
        // the first pass has a catalog to match stands against.
        jobs::spawn_departure_runway_derive(
            state.jobs.clone(),
            pool.clone(),
            state.feed.clone(),
            state.gates.clone(),
        );
        // Load configurable aircraft performance profiles and keep them current for the ETA model.
        jobs::spawn_aircraft_profiles_refresh(
            state.jobs.clone(),
            pool.clone(),
            state.aircraft_profiles.clone(),
        );
        // ATC sector volumes (#594), imported offline; kept as a dataset when the Monitor went (#719).
        jobs::spawn_airspace_sectors_refresh(
            state.jobs.clone(),
            pool.clone(),
            state.airspace_sectors.clone(),
        );
        // Sector occupancy limit overrides (#722); the handler also force-reloads on write.
        jobs::spawn_sector_limits_refresh(
            state.jobs.clone(),
            pool.clone(),
            state.sector_limits.clone(),
        );
        // Sector consolidations (#723); the handler also force-reloads on write.
        jobs::spawn_sector_consolidations_refresh(
            state.jobs.clone(),
            pool.clone(),
            state.sector_consolidations.clone(),
        );
        // Airport surface gates, for feed::taxi_observations's gate matching (kept DB-less).
        jobs::spawn_airport_gates_refresh(state.jobs.clone(), pool.clone(), state.gates.clone());
        // Manually excluded ("bogus") flights, for the DB-less flow surfaces (#342). Also runs the
        // auto-clear once a callsign leaves the feed.
        jobs::spawn_flight_exclusions_refresh(
            state.jobs.clone(),
            pool.clone(),
            state.feed.clone(),
            state.flight_exclusions.clone(),
        );
        // Seed airport ramp/taxiway geometry from the bundled FAA AM extract (#230/#231).
        jobs::spawn_faa_surface_seed(state.jobs.clone(), pool.clone());
        // Seed airport parking stands from the bundled X-Plane extract (#431). After
        // spawn_airport_gates_refresh above, so a seed's own cache reload is not immediately
        // overwritten by a poll that started before the rows landed.
        jobs::spawn_xplane_gate_seed(state.jobs.clone(), pool.clone(), state.gates.clone());
        // Learned taxi-observation samples, for feed::flow's ground-allowance estimate (#164
        // sub-issue E, kept DB-less).
        jobs::spawn_taxi_estimate_samples_refresh(
            state.jobs.clone(),
            pool.clone(),
            state.taxi_estimate_samples.clone(),
        );
        feed::events::spawn_sync(pool.clone());
        // Persistent stats collection off the shared feed snapshot + its retention compaction.
        feed::stats::spawn_collector(pool.clone(), state.feed.clone(), state.airspace.clone());
        // Per-flight delay legs (taxi-out + arrival transit) for the average-delay page.
        feed::delays::spawn_collector(pool.clone(), state.feed.clone(), state.runways.clone());
        // Per-gate/type/runway pushback+taxi-out observations (#164 sub-issue C).
        feed::taxi_observations::spawn_collector(
            pool.clone(),
            state.feed.clone(),
            state.runways.clone(),
            state.gates.clone(),
        );
        jobs::spawn_stats_compaction(state.jobs.clone(), pool.clone());
        jobs::spawn_capture_scheduler(state.jobs.clone(), pool.clone());
        // Event FCAs + TMI packages: auto-publish 30 min before start, auto-archive at end.
        jobs::spawn_event_fca_lifecycle(state.jobs.clone(), pool.clone(), state.events.clone());
        jobs::spawn_event_package_lifecycle(state.jobs.clone(), pool.clone(), state.events.clone());
        // ACE-claim reminder DMs at T-24h/T-6h before the event.
        jobs::spawn_ace_reminder_scheduler(state.jobs.clone(), pool.clone(), state.events.clone());
        // VATUSA: register the division webhook, and pull the whole division daily (#605).
        feed::vatusa::spawn_register_webhook(pool.clone());
        feed::vatusa::spawn_division_pull(state.jobs.clone(), pool, state.events.clone());
    }

    let limits = std::sync::Arc::new(rate_limit::RateLimits::from_env());
    rate_limit::spawn_cleanup(limits.clone());
    // Per-credential request volume, shown where keys are managed (#611).
    if let Some(pool) = state.db.clone() {
        jobs::spawn_credential_usage_flush(state.jobs.clone(), pool, limits.clone());
    }
    let app = router::build_router_with_limits(state, limits);

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
        .parse()?;

    tracing::info!(%addr, version = VERSION, "starting ois backend");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into());
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}

async fn run_startup_migrations(
    state: &state::AppState,
) -> Result<(), sqlx::migrate::MigrateError> {
    let Some(pool) = state.db.as_ref() else {
        tracing::info!("startup migrations skipped (no database configured)");
        return Ok(());
    };
    tracing::info!("running startup migrations");
    sqlx::migrate!("./migrations").run(pool).await
}

/// Demote every server admin no longer in `OIS_SERVER_ADMIN_CID` before serving (#805). After the
/// migrations, so the rows 0130 re-tagged are in place. A failure is logged and boot goes on: sign-in
/// still demotes, so failing leaves only the wait for the removed admin's next sign-in.
async fn demote_removed_server_admins(state: &state::AppState) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    let admin_cids = config::configured_server_admin_cids();
    if let Err(error) = handlers::auth::demote_unconfigured_server_admins(pool, &admin_cids).await {
        tracing::error!(
            ?error,
            "could not demote server admins removed from OIS_SERVER_ADMIN_CID at startup"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    /// Not a behavior test — a guardrail. `#[sqlx::test]` applies every embedded migration (in
    /// order) against a fresh database as its own setup step before the body runs; reaching this
    /// line at all is the assertion. Otherwise a broken/out-of-order migration is only discovered
    /// when a real backend boots against a real DB.
    ///
    /// It also catches two migrations sharing a version, once both are in the tree: sqlx applies both
    /// and the second violates `_sqlx_migrations`' primary key. That failure is a constraint dump,
    /// though, so `migration_versions_are_unique` below exists to say the same thing readably (#569).
    #[sqlx::test]
    async fn migrations_apply_cleanly(_pool: sqlx::PgPool) {}

    /// Startup demotes the server admins removed from the configured list, after the migrations and
    /// before serving (#805). A source scan, because a test cannot set `OIS_SERVER_ADMIN_CID` without
    /// racing every other test, nor boot `run()`.
    #[test]
    fn startup_demotes_server_admins_removed_from_the_list() {
        let source = include_str!("lib.rs");
        let run = &source[source.find("pub async fn run()").unwrap()..];
        let run = &run[..run.find("\n}\n").unwrap()];
        let migrations = run.find("run_startup_migrations(&state).await?;").unwrap();
        let demote = run
            .find("demote_removed_server_admins(&state).await;")
            .expect("run() demotes removed server admins");
        let serve = run.find("axum::serve(").unwrap();
        assert!(migrations < demote && demote < serve);

        let pass = &source[source
            .find("async fn demote_removed_server_admins")
            .unwrap()..];
        let pass = &pass[..pass.find("\n}\n").unwrap()];
        assert!(pass.contains("let admin_cids = config::configured_server_admin_cids();"));
        assert!(pass.contains("demote_unconfigured_server_admins(pool, &admin_cids)"));
    }

    /// Every version claimed by more than one migration file, with the files that claim it.
    ///
    /// The version is the prefix before the first `_`, which is how sqlx reads a
    /// `NNNN_name.sql` filename.
    fn duplicate_versions<'a>(
        names: impl IntoIterator<Item = &'a str>,
    ) -> Vec<(String, Vec<String>)> {
        let mut by_version: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for name in names.into_iter().filter(|n| n.ends_with(".sql")) {
            let version = name.split('_').next().unwrap_or(name).to_string();
            by_version
                .entry(version)
                .or_default()
                .push(name.to_string());
        }
        by_version
            .into_iter()
            .filter(|(_, files)| files.len() > 1)
            .collect()
    }

    /// Two migrations with the same version do not fail cleanly. sqlx sorts without deduplicating
    /// and snapshots the applied set before the apply loop, so **both** run, the second violates
    /// `_sqlx_migrations`' primary key, and startup leaves the database half-migrated.
    ///
    /// A collision usually arrives when two open PRs each pick "the next free number" and both merge
    /// (#569 — three in one evening). Neither branch has a duplicate on its own, so this fires on the
    /// merged tree: the push-to-`next` CI run, or `just ci` after pulling. Unlike the PK violation
    /// above, it needs no database and names the files to renumber. A gap in the sequence is
    /// harmless; only a repeat is fatal.
    #[test]
    fn migration_versions_are_unique() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("backend/migrations is readable")
            .map(|e| {
                e.expect("a directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        // `read_dir` order is unspecified; sorted so a failure names the files the same way every run.
        names.sort();
        let dupes = duplicate_versions(names.iter().map(String::as_str));

        assert!(
            dupes.is_empty(),
            "{}",
            dupes
                .iter()
                .map(|(v, files)| format!(
                    "duplicate migration version {v}: {} — renumber all but one above every \
                     open PR's highest (see AGENTS.md § Migrations)",
                    files.join(", ")
                ))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// The check above passes vacuously if this helper stops finding duplicates, so it is exercised
    /// directly: a real repeat, a harmless gap, and a name containing a second `_`.
    #[test]
    fn the_duplicate_check_notices_a_duplicate() {
        let names = [
            "0089_departure_runway.sql",
            "0090_departure_runway_rules.sql",
            "0090_some_other_thing.sql",
            "0093_after_a_gap.sql",
            "README.md",
        ];

        assert_eq!(
            duplicate_versions(names),
            vec![(
                "0090".to_string(),
                vec![
                    "0090_departure_runway_rules.sql".to_string(),
                    "0090_some_other_thing.sql".to_string(),
                ],
            )],
            "only the repeated 0090 is a duplicate; the 0091-0092 gap and non-SQL files are not"
        );
    }
}

#[cfg(test)]
mod honolulu_migration_tests {
    //! VATUSA/OIS#556: `0101_honolulu_is_hcf.sql` repoints stored `'ZHN'` to `'HCF'`. It finds its
    //! columns from `information_schema` at deploy time, so the test builds a table of its own to drive
    //! every branch — including the unique-collision fallback — and uses one real table to show the
    //! discovery reaches the live schema.

    const MIGRATION: &str = include_str!("../migrations/0101_honolulu_is_hcf.sql");

    #[sqlx::test]
    async fn stored_zhn_becomes_hcf_and_a_collision_is_left_alone(pool: sqlx::PgPool) {
        // A table shaped like the risky ones: an ARTCC column inside a unique key.
        sqlx::query(
            "create table public.t556 (k int not null, artcc text not null, note text, \
             unique (k, artcc))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into public.t556 (k, artcc, note) values \
             (1, 'ZHN', 'plain'), \
             (2, 'ZHN', 'collides'), (2, 'HCF', 'twin'), \
             (3, 'ZDC', 'untouched')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // A real table, so the information_schema discovery is shown to reach the actual schema.
        // `identity.vatusa_roles.facility` is free text with no FK, and unlike the per-facility
        // `vatusa_webhooks` this test used to pick, it survives 0104's division-webhook rework.
        sqlx::query(
            "insert into identity.vatusa_roles (cid, facility, role) values (556556, 'ZHN', 'ATM')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(MIGRATION)
            .execute(&pool)
            .await
            .expect("the migration never fails startup");

        let rows: Vec<(String, String)> =
            sqlx::query_as("select note, artcc from public.t556 order by k, note")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            [
                ("plain".into(), "HCF".into()),
                ("collides".into(), "ZHN".into()),
                ("twin".into(), "HCF".into()),
                ("untouched".into(), "ZDC".into()),
            ],
            "the plain row moves, the colliding one stays beside its twin, other ids are untouched"
        );
        let stored: String =
            sqlx::query_scalar("select facility from identity.vatusa_roles where cid = 556556")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, "HCF", "a real column, found by discovery");
    }
}
