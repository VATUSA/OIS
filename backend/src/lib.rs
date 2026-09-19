pub mod audit;
pub mod auth;
pub mod config;
pub mod errors;
pub mod feed;
pub mod handlers;
pub mod job_registry;
pub mod jobs;
pub mod models;
pub mod openapi;
pub mod realtime;
pub mod repos;
pub mod reqlog;
pub mod router;
#[cfg(test)]
pub(crate) mod scope_test_support;
pub mod state;
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

    feed::spawn_poller(state.feed.clone());
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
        jobs::spawn_cleanup(state.jobs.clone(), pool.clone());
        // Load configurable aircraft performance profiles and keep them current for the ETA model.
        jobs::spawn_aircraft_profiles_refresh(
            state.jobs.clone(),
            pool.clone(),
            state.aircraft_profiles.clone(),
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
        jobs::spawn_ace_reminder_scheduler(state.jobs.clone(), pool.clone());
        // VATUSA member sync: register the roster-change webhook and periodically reconcile.
        feed::vatusa::spawn_register_webhooks(pool.clone());
        feed::vatusa::spawn_reconcile(pool);
    }

    let app = router::build_router(state);

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

#[cfg(test)]
mod tests {
    /// Not a behavior test — a guardrail. `#[sqlx::test]` applies every embedded migration (in
    /// order) against a fresh database as its own setup step before the body runs; reaching this
    /// line at all is the assertion. Otherwise a broken/out-of-order migration is only discovered
    /// when a real backend boots against a real DB.
    #[sqlx::test]
    async fn migrations_apply_cleanly(_pool: sqlx::PgPool) {}
}
