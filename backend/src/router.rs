use axum::{
    Router, middleware,
    routing::{delete, get, patch, post, put},
};

use crate::{
    auth::middleware::resolve_current_user,
    config::build_cors_layer,
    handlers::{
        access, audit, auth, docs, events, facilities, feed, health, service_accounts, tmu, users,
    },
    state::AppState,
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/docs/api/v1/openapi.json", get(docs::openapi_json))
        .route("/api/v1/auth/vatsim/login", get(auth::vatsim_login))
        .route("/api/v1/auth/vatsim/callback", get(auth::vatsim_callback))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/me", get(auth::me))
        // User directory search
        .route("/api/v1/users", get(users::search_users))
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
        // TMU — Traffic Management Initiatives
        .route(
            "/api/v1/tmu/tmis",
            get(tmu::list_tmis).post(tmu::create_tmi),
        )
        .route(
            "/api/v1/tmu/tmis/{id}",
            patch(tmu::update_tmi).delete(tmu::delete_tmi),
        )
        .route("/api/v1/tmu/tmis/{id}/publish", post(tmu::publish_tmi))
        .route("/api/v1/tmu/tmis/{id}/cancel", post(tmu::cancel_tmi))
        // TMU — ground stops
        .route(
            "/api/v1/tmu/ground-stops",
            get(tmu::list_ground_stops).post(tmu::create_ground_stop),
        )
        .route(
            "/api/v1/tmu/ground-stops/{id}",
            delete(tmu::delete_ground_stop),
        )
        .route(
            "/api/v1/tmu/ground-stops/{id}/publish",
            post(tmu::publish_ground_stop),
        )
        .route(
            "/api/v1/tmu/ground-stops/{id}/cancel",
            post(tmu::cancel_ground_stop),
        )
        // TMU — airport rate programs
        .route("/api/v1/tmu/programs", get(tmu::list_programs))
        .route(
            "/api/v1/tmu/programs/{icao}",
            put(tmu::upsert_program).delete(tmu::delete_program),
        )
        // Events (VATUSA cache — anchors per-event planning)
        .route("/api/v1/events", get(events::list_events))
        .route("/api/v1/events/{id}", get(events::get_event))
        .route(
            "/api/v1/events/{id}/dcc",
            get(events::get_event_dcc).put(events::update_event_dcc),
        )
        // Live VATSIM feed
        .route("/api/v1/feed/status", get(feed::feed_status))
        .route("/api/v1/tmu/flow/{icao}", get(feed::airport_flow))
        .route("/api/v1/tmu/departures/{dep}", get(feed::list_departures))
        .route("/api/v1/tmu/taxi/{icao}", get(feed::taxi_stats))
        .route("/api/v1/tmu/cfr", post(feed::issue_cfr))
        .route("/api/v1/tmu/cfr/{callsign}", delete(feed::release_cfr))
        // Audit log
        .route("/api/v1/admin/audit", get(audit::list_audit_logs))
        // Service accounts (bot credentials)
        .route(
            "/api/v1/admin/service-accounts",
            get(service_accounts::list_service_accounts)
                .post(service_accounts::create_service_account),
        )
        .route(
            "/api/v1/admin/service-accounts/{id}/rotate",
            post(service_accounts::rotate_service_account),
        )
        .route(
            "/api/v1/admin/service-accounts/{id}/disable",
            post(service_accounts::disable_service_account),
        )
        .route(
            "/api/v1/admin/service-accounts/{id}/roles",
            put(service_accounts::set_service_account_roles),
        )
        // Runs before handlers so CurrentUser / service account are in extensions.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            resolve_current_user,
        ))
        .layer(build_cors_layer())
        .with_state(state)
}
