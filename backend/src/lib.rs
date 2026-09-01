pub mod audit;
pub mod auth;
pub mod config;
pub mod errors;
pub mod feed;
pub mod handlers;
pub mod jobs;
pub mod models;
pub mod openapi;
pub mod realtime;
pub mod repos;
pub mod reqlog;
pub mod router;
pub mod state;

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
    jobs::spawn_nav_refresh(state.nav.clone(), state.nav_refreshed.clone());
    jobs::spawn_winds_refresh(
        state.feed.clone(),
        state.winds.clone(),
        state.winds_refreshed.clone(),
        state.db.clone(),
    );
    if let Some(pool) = state.db.clone() {
        jobs::spawn_cleanup(pool.clone());
        feed::events::spawn_sync(pool.clone());
        // Persistent stats collection off the shared feed snapshot + its retention compaction.
        feed::stats::spawn_collector(pool.clone(), state.feed.clone(), state.airspace.clone());
        // Per-flight delay legs (taxi-out + arrival transit) for the average-delay page.
        feed::delays::spawn_collector(pool.clone(), state.feed.clone(), state.runways.clone());
        jobs::spawn_stats_compaction(pool.clone());
        jobs::spawn_capture_scheduler(pool.clone());
        // Event FCAs + TMI packages: auto-publish 30 min before start, auto-archive at end.
        jobs::spawn_event_fca_lifecycle(pool.clone(), state.events.clone());
        jobs::spawn_event_package_lifecycle(pool.clone(), state.events.clone());
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
