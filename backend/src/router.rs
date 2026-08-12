use axum::{
    Router, middleware,
    routing::{get, post},
};

use crate::{
    auth::middleware::resolve_current_user,
    config::build_cors_layer,
    handlers::{access, auth, facilities, health},
    state::AppState,
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/api/v1/auth/vatsim/login", get(auth::vatsim_login))
        .route("/api/v1/auth/vatsim/callback", get(auth::vatsim_callback))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/me", get(auth::me))
        // Facilities (ARTCC directory) — public reference data
        .route("/api/v1/facilities", get(facilities::list_facilities))
        .route("/api/v1/facilities/{id}", get(facilities::get_facility))
        // Access editor
        .route("/api/v1/access/catalog", get(access::get_access_catalog))
        .route("/api/v1/access/self", get(access::get_self_access))
        .route(
            "/api/v1/admin/users/{cid}/access",
            get(access::get_user_access).post(access::update_user_access),
        )
        // Runs before handlers so CurrentUser / service account are in extensions.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            resolve_current_user,
        ))
        .layer(build_cors_layer())
        .with_state(state)
}
