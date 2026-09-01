use axum::{
    Router, middleware,
    routing::{delete, get, patch, post, put},
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    auth::middleware::resolve_current_user,
    config::build_cors_layer,
    handlers::{
        access, ace, airport_configs, api_keys, atc, audit, auth, dashboards, docs, events,
        facilities, facility_map, feed, flow, gdp, health, integration, preferences, public,
        runway, service_accounts, stats, tmu, users, webhooks,
    },
    openapi::ApiDoc,
    realtime,
    state::AppState,
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/docs/api/v1/openapi.json", get(docs::openapi_json))
        // Interactive API docs (Swagger UI), served from the same generated spec. Try-it-out calls
        // hit the real endpoints and obey their auth (session cookie or bearer token).
        .merge(SwaggerUi::new("/docs/swagger").url("/docs/swagger/openapi.json", ApiDoc::openapi()))
        .route("/api/v1/auth/vatsim/login", get(auth::vatsim_login))
        .route("/api/v1/auth/vatsim/callback", get(auth::vatsim_callback))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/me", get(auth::me))
        .route(
            "/api/v1/me/preferences/{namespace}",
            get(preferences::get_preferences).put(preferences::put_preferences),
        )
        // Dashboards — multiple named boards per user, collections, share-by-slug.
        .route(
            "/api/v1/dashboards",
            get(dashboards::list_dashboards).post(dashboards::create_dashboard),
        )
        .route(
            "/api/v1/dashboards/{id}",
            get(dashboards::get_dashboard)
                .put(dashboards::update_dashboard)
                .delete(dashboards::delete_dashboard),
        )
        .route(
            "/api/v1/dashboards/{id}/share",
            post(dashboards::share_dashboard).delete(dashboards::unshare_dashboard),
        )
        .route(
            "/api/v1/dashboards/shared/{slug}",
            get(dashboards::get_shared_dashboard),
        )
        .route(
            "/api/v1/dashboards/shared/{slug}/copy",
            post(dashboards::copy_shared_dashboard),
        )
        .route(
            "/api/v1/dashboard-collections",
            post(dashboards::create_collection),
        )
        .route(
            "/api/v1/dashboard-collections/{id}",
            put(dashboards::rename_collection).delete(dashboards::delete_collection),
        )
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
        // Inbound VATUSA roster-change webhook — no session; verified by HMAC signature.
        .route(
            "/api/v1/webhooks/vatusa/{facility}",
            post(webhooks::vatusa_webhook),
        )
        // Access editor
        .route("/api/v1/access/catalog", get(access::get_access_catalog))
        .route("/api/v1/access/self", get(access::get_self_access))
        .route("/api/v1/admin/users", get(access::list_users))
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
        .route(
            "/api/v1/events/{id}/capture",
            get(events::get_event_capture).put(events::update_event_capture),
        )
        .route("/api/v1/events/{id}/stats", get(events::get_event_stats))
        .route(
            "/api/v1/events/{id}/availability",
            get(events::get_event_availability),
        )
        .route(
            "/api/v1/events/{id}/discord/publish",
            post(events::publish_event_discord),
        )
        .route(
            "/api/v1/events/{id}/debrief",
            get(events::get_event_debrief).put(events::update_event_debrief),
        )
        .route("/api/v1/events/{id}/rates", get(events::list_event_rates))
        .route(
            "/api/v1/events/{id}/rates/{icao}",
            put(events::upsert_event_rate).delete(events::delete_event_rate),
        )
        // ACE support — event-scoped requests + per-person slot claims (notes + times)
        .route(
            "/api/v1/events/{id}/ace",
            get(ace::list_requests).post(ace::create_request),
        )
        // Fan out Tier-1 neighbour requests for an FNO (must precede the `{req}` route).
        .route("/api/v1/events/{id}/ace/tier1", post(ace::generate_tier1))
        .route("/api/v1/events/{id}/ace/{req}", delete(ace::delete_request))
        .route(
            "/api/v1/events/{id}/ace/{req}/claim",
            post(ace::claim_request).delete(ace::release_claim),
        )
        .route(
            "/api/v1/events/{id}/ace/{req}/decide",
            post(ace::decide_request),
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
        .route(
            "/api/v1/events/{id}/packages/{package_id}/deactivate",
            post(events::deactivate_event_package),
        )
        .route(
            "/api/v1/events/{id}/packages/{package_id}/auto",
            put(events::set_event_package_auto),
        )
        // Event-specific FCAs — planned in the event manager, invisible on live maps until published
        .route(
            "/api/v1/events/{id}/fcas",
            get(events::list_event_fcas).post(events::create_event_fca),
        )
        .route(
            "/api/v1/events/{id}/fcas/{fca_id}",
            put(events::update_event_fca).delete(events::delete_event_fca),
        )
        .route(
            "/api/v1/events/{id}/fcas/{fca_id}/publish",
            post(events::publish_event_fca),
        )
        .route(
            "/api/v1/events/{id}/fcas/{fca_id}/archive",
            post(events::archive_event_fca),
        )
        .route(
            "/api/v1/events/{id}/fcas/{fca_id}/auto",
            put(events::set_event_fca_auto),
        )
        // Reusable per-airport runway configs (default AAR/ADR + wind rule) for event planning
        .route(
            "/api/v1/airport-configs/{icao}",
            get(airport_configs::list_airport_configs).post(airport_configs::create_airport_config),
        )
        .route(
            "/api/v1/airport-configs/{icao}/{id}",
            put(airport_configs::update_airport_config)
                .delete(airport_configs::delete_airport_config),
        )
        // Airport wind forecast (Open-Meteo) for event-day ops prediction
        .route(
            "/api/v1/forecast/{icao}",
            get(airport_configs::forecast_wind),
        )
        // Persisted VATSIM stats (historical read API)
        .route("/api/v1/stats/network/history", get(stats::network_history))
        .route("/api/v1/stats/airports/top", get(stats::airports_top))
        .route("/api/v1/stats/delays", get(stats::delay_summary))
        .route("/api/v1/stats/airports/{icao}", get(stats::airport_stats))
        .route(
            "/api/v1/stats/airports/{icao}/movements",
            get(stats::airport_movements),
        )
        .route(
            "/api/v1/stats/members/{cid}/flights",
            get(stats::member_flights),
        )
        .route("/api/v1/stats/flights/{id}", get(stats::flight_detail))
        .route("/api/v1/stats/flights/{id}/track", get(stats::flight_track))
        .route("/api/v1/stats/captures", get(stats::list_captures))
        .route(
            "/api/v1/stats/captures/{id}/replay",
            get(stats::capture_replay),
        )
        .route("/api/v1/stats/replay", get(stats::window_replay))
        .route("/api/v1/stats/replay/positions", get(stats::replay_chunk))
        // Historical ("time-machine") dashboard: feed compute functions replayed at instant T
        .route("/api/v1/stats/hist/flow/{icao}", get(stats::hist_flow))
        .route(
            "/api/v1/stats/hist/departures/{dep}",
            get(stats::hist_departures),
        )
        .route("/api/v1/stats/hist/atc", get(stats::hist_atc))
        .route("/api/v1/stats/hist/traffic", get(stats::hist_traffic))
        .route("/api/v1/stats/hist/runway/{icao}", get(stats::hist_runway))
        .route("/api/v1/stats/hist/taxi/{icao}", get(stats::hist_taxi))
        .route("/api/v1/stats/hist/fcas", get(stats::hist_fcas))
        .route("/api/v1/stats/hist/tmis", get(stats::hist_tmis))
        .route("/api/v1/stats/hist/gdps", get(stats::hist_gdps))
        .route(
            "/api/v1/stats/hist/ground-stops",
            get(stats::hist_ground_stops),
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
        .route("/api/v1/flow/resolve-routes", post(flow::resolve_routes))
        .route("/api/v1/flow/traffic", get(flow::list_traffic))
        .route("/api/v1/flow/idst", get(flow::list_idst))
        // Realtime push — additive over REST (see crate::realtime).
        .route("/api/v1/ws", get(realtime::ws))
        .route("/api/v1/flow/atc", get(atc::list_atc))
        .route("/api/v1/flow/facilities", get(atc::list_flow_facilities))
        // Facility map color rules — public read, facility-scoped write.
        .route(
            "/api/v1/facility-map/{id}/config",
            get(facility_map::get_config).put(facility_map::put_config),
        )
        .route("/api/v1/flow/data-status", get(flow::data_status))
        .route("/api/v1/flow/data-refresh", post(flow::data_refresh))
        .route("/api/v1/flow/route-coverage", get(flow::route_coverage))
        .route("/api/v1/flow/validate-fixes", get(flow::validate_fixes))
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
        // Discord integration — outbound-job queue (bot) + guild config
        .route(
            "/api/v1/integration/jobs/lease",
            post(integration::lease_jobs),
        )
        .route(
            "/api/v1/integration/jobs/{id}/ack",
            post(integration::ack_job),
        )
        .route(
            "/api/v1/integration/discord",
            get(integration::get_discord_config).put(integration::put_discord_config),
        )
        .route(
            "/api/v1/integration/discord/guilds/snapshot",
            post(integration::push_guild_snapshot),
        )
        .route(
            "/api/v1/integration/discord/refresh",
            post(integration::refresh_guild_snapshot),
        )
        .route(
            "/api/v1/integration/discord/ace/{id}/claim",
            post(integration::discord_ace_claim),
        )
        .route(
            "/api/v1/integration/discord/ace/{id}",
            get(integration::discord_ace_info),
        )
        .route(
            "/api/v1/integration/discord/availability/{id}",
            post(integration::discord_availability),
        )
        // The current user's Discord link (read-only; sourced from VATUSA)
        .route("/api/v1/me/discord", get(integration::get_my_discord))
        // API keys (user-owned personal access tokens) — self-service
        .route(
            "/api/v1/api-keys",
            get(api_keys::list_my_keys).post(api_keys::create_key),
        )
        .route(
            "/api/v1/api-keys/grantable-permissions",
            get(api_keys::grantable_permissions),
        )
        .route(
            "/api/v1/api-keys/{id}",
            get(api_keys::get_my_key).delete(api_keys::delete_my_key),
        )
        .route("/api/v1/api-keys/{id}/rotate", post(api_keys::rotate_key))
        .route(
            "/api/v1/api-keys/{id}/disable",
            post(api_keys::disable_my_key),
        )
        .route(
            "/api/v1/api-keys/{id}/permissions",
            put(api_keys::set_key_permissions),
        )
        .route("/api/v1/api-keys/{id}/audit", get(api_keys::key_audit))
        // API keys — admin oversight (any user's keys)
        .route("/api/v1/admin/api-keys", get(api_keys::admin_list_keys))
        .route(
            "/api/v1/admin/api-keys/{id}/disable",
            post(api_keys::admin_disable_key),
        )
        .route(
            "/api/v1/admin/api-keys/{id}",
            delete(api_keys::admin_delete_key),
        )
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
