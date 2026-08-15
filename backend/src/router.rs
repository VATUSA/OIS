use axum::{
    Router, middleware,
    routing::{delete, get, patch, post, put},
};

use crate::{
    auth::middleware::resolve_current_user,
    config::build_cors_layer,
    handlers::{
        access, atc, audit, auth, docs, events, facilities, feed, flow, gdp, health, public,
        runway, service_accounts, tmu, users,
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
        // Public advisories — read-only, no auth (active TMIs). The FCA overview
        // reuses the now-public GET /api/v1/flow/fcas via the shared map.
        .route("/api/v1/public/board", get(public::get_board))
        .route(
            "/api/v1/public/flight/{callsign}",
            get(flow::flight_advisory),
        )
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
        // TMU — Ground Delay Programs (GDP)
        .route("/api/v1/tmu/gdp", get(gdp::list_gdps).post(gdp::create_gdp))
        .route(
            "/api/v1/tmu/gdp/{id}",
            put(gdp::revise_gdp).delete(gdp::delete_gdp),
        )
        .route("/api/v1/tmu/gdp/{id}/board", get(gdp::get_gdp_board))
        .route("/api/v1/tmu/gdp/{id}/publish", post(gdp::publish_gdp))
        .route("/api/v1/tmu/gdp/{id}/cancel", post(gdp::cancel_gdp))
        .route("/api/v1/tmu/gdp/{id}/compress", post(gdp::compress_gdp))
        .route(
            "/api/v1/tmu/gdp/{id}/slots/{callsign}",
            post(gdp::lock_slot).delete(gdp::unlock_slot),
        )
        // Events (VATUSA cache — anchors per-event planning)
        .route("/api/v1/events", get(events::list_events))
        .route("/api/v1/events/{id}", get(events::get_event))
        .route(
            "/api/v1/events/{id}/dcc",
            get(events::get_event_dcc).put(events::update_event_dcc),
        )
        .route(
            "/api/v1/events/{id}/facilities",
            get(events::list_event_facilities),
        )
        .route(
            "/api/v1/events/{id}/facilities/{facility}",
            put(events::upsert_event_facility).delete(events::delete_event_facility),
        )
        .route("/api/v1/events/{id}/rates", get(events::list_event_rates))
        .route(
            "/api/v1/events/{id}/rates/{icao}",
            put(events::upsert_event_rate).delete(events::delete_event_rate),
        )
        .route(
            "/api/v1/events/{id}/staffing",
            get(events::list_event_staffing),
        )
        .route(
            "/api/v1/events/{id}/staffing/{facility}",
            put(events::upsert_event_staffing).delete(events::delete_event_staffing),
        )
        .route(
            "/api/v1/events/{id}/packages",
            get(events::list_event_packages).post(events::create_event_package),
        )
        .route(
            "/api/v1/events/{id}/packages/{package_id}",
            delete(events::delete_event_package),
        )
        .route(
            "/api/v1/events/{id}/packages/{package_id}/items",
            post(events::add_event_package_item),
        )
        .route(
            "/api/v1/events/{id}/packages/{package_id}/items/{item_id}",
            delete(events::delete_event_package_item),
        )
        .route(
            "/api/v1/events/{id}/packages/{package_id}/activate",
            post(events::activate_event_package),
        )
        // Flow constrained areas (FCAs) + live map traffic
        .route(
            "/api/v1/flow/fcas",
            get(flow::list_fcas).post(flow::create_fca),
        )
        .route(
            "/api/v1/flow/fcas/{id}",
            put(flow::update_fca).delete(flow::delete_fca),
        )
        .route("/api/v1/flow/fcas/{id}/traffic", get(flow::fca_traffic))
        .route("/api/v1/flow/fcas/{id}/order", put(flow::reorder_fca))
        .route(
            "/api/v1/flow/fcas/{id}/release/{callsign}",
            post(flow::mark_release).delete(flow::clear_release),
        )
        // Shared named map routes (polylines)
        .route(
            "/api/v1/flow/routes",
            get(flow::list_routes).post(flow::create_route),
        )
        .route(
            "/api/v1/flow/routes/{id}",
            put(flow::update_route).delete(flow::delete_route),
        )
        .route("/api/v1/flow/counts", get(flow::fca_counts))
        .route(
            "/api/v1/flow/aircraft/{callsign}/route",
            get(flow::aircraft_route),
        )
        .route("/api/v1/flow/traffic", get(flow::list_traffic))
        .route("/api/v1/flow/atc", get(atc::list_atc))
        .route("/api/v1/flow/data-status", get(flow::data_status))
        .route("/api/v1/flow/data-refresh", post(flow::data_refresh))
        .route("/api/v1/flow/route-coverage", get(flow::route_coverage))
        .route(
            "/api/v1/flow/runway/{icao}",
            get(runway::get_runway).put(runway::put_runway),
        )
        .route(
            "/api/v1/flow/runway/{icao}/configs",
            get(runway::list_saved_configs),
        )
        .route(
            "/api/v1/flow/runway/{icao}/configs/{name}",
            put(runway::save_config).delete(runway::delete_config),
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
        // Innermost app layer: records every successful mutation to the audit log. Added
        // before resolve_current_user so it runs *after* it inbound and sees CurrentUser.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::audit::audit_mutations,
        ))
        // Dev request log — one line per request. Outside audit (so its latency covers the
        // whole request), inside resolve_current_user (so it can name the actor).
        .layer(middleware::from_fn(crate::reqlog::log_requests))
        // Runs before handlers so CurrentUser / service account are in extensions.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            resolve_current_user,
        ))
        .layer(build_cors_layer())
        .with_state(state)
}
