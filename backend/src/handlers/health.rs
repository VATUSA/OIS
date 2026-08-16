use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::state::AppState;

/// Liveness + DB reachability probe.
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses((status = 200, description = "Liveness + DB reachability"))
)]
pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let db_ok = match state.db.as_ref() {
        Some(pool) => sqlx::query("select 1").execute(pool).await.is_ok(),
        None => false,
    };
    Json(json!({ "status": "ok", "database": db_ok, "version": crate::VERSION }))
}
