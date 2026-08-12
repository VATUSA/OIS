pub mod auth;
pub mod config;
pub mod errors;
pub mod handlers;
pub mod models;
pub mod openapi;
pub mod repos;
pub mod router;
pub mod state;

use std::net::SocketAddr;

use tracing_subscriber::{EnvFilter, fmt};

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    init_tracing();

    let state = state::AppState::from_env().await?;
    run_startup_migrations(&state).await?;

    let app = router::build_router(state);

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
        .parse()?;

    tracing::info!(%addr, "starting ois backend");

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
