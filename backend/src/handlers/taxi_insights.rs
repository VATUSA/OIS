//! Read-only staff endpoints for #183: browse raw taxi/pushback observations and their derived
//! estimates. Gated on `StatsRead`, same as every other `/api/v1/stats/*` browsing endpoint — this
//! data lives in the same `stats` schema and warrants no finer-grained permission.

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    auth::{permissions::StatsRead, require_permission::RequirePermission},
    errors::ApiError,
    feed::taxi_estimate::EstimateTier,
    models::{TaxiEstimatePage, TaxiObservationPage},
    repos::taxi_insights::{self as repo, EstimateFilters, ObservationFilters},
    state::AppState,
};

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

fn norm_opt(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_ascii_uppercase())
        .filter(|v| !v.is_empty())
}

#[derive(Deserialize)]
pub struct ObservationsQuery {
    airport: Option<String>,
    gate_id: Option<String>,
    aircraft: Option<String>,
    runway: Option<String>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    include_outliers: Option<bool>,
    page: Option<i64>,
    page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/taxi/observations",
    tag = "stats",
    params(
        ("airport" = Option<String>, Query, description = "Filter to one airport ICAO"),
        ("gate_id" = Option<String>, Query, description = "Filter to one gate/parking spot"),
        ("aircraft" = Option<String>, Query, description = "Filter to one aircraft type"),
        ("runway" = Option<String>, Query, description = "Filter to one departure runway"),
        ("from" = Option<String>, Query, description = "Only observed at/after (RFC 3339)"),
        ("to" = Option<String>, Query, description = "Only observed at/before (RFC 3339)"),
        ("include_outliers" = Option<bool>, Query, description = "Include rows outside taxi_estimate's sanity bounds (default true)"),
        ("page" = Option<i64>, Query, description = "1-based page (default 1)"),
        ("page_size" = Option<i64>, Query, description = "Page size (default 50, max 100)")
    ),
    responses((status = 200, body = TaxiObservationPage), (status = 401))
)]
pub async fn list_taxi_observations(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<ObservationsQuery>,
) -> Result<Json<TaxiObservationPage>, ApiError> {
    let p = pool(&state)?;
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 100);
    let filters = ObservationFilters {
        airport: norm_opt(q.airport),
        gate_id: norm_opt(q.gate_id),
        aircraft: norm_opt(q.aircraft),
        runway: norm_opt(q.runway),
        from: q.from,
        to: q.to,
        include_outliers: q.include_outliers.unwrap_or(true),
        limit: page_size,
        offset: (page - 1) * page_size,
    };
    let total = repo::count_taxi_observations(p, &filters).await?;
    let items = repo::fetch_taxi_observations(p, &filters).await?;
    Ok(Json(TaxiObservationPage {
        items,
        total,
        page,
        page_size,
    }))
}

#[derive(Deserialize)]
pub struct EstimatesQuery {
    airport: String,
    gate_id: Option<String>,
    aircraft: Option<String>,
    runway: Option<String>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    include_outliers: Option<bool>,
    /// `gate_type_runway` | `airport_runway` | `airport` | `default`.
    fallback_tier: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/stats/taxi/estimates",
    tag = "stats",
    params(
        ("airport" = String, Query, description = "Airport ICAO (required — estimates are computed per-airport)"),
        ("gate_id" = Option<String>, Query, description = "Filter to one gate/parking spot"),
        ("aircraft" = Option<String>, Query, description = "Filter to one aircraft type"),
        ("runway" = Option<String>, Query, description = "Filter to one departure runway"),
        ("from" = Option<String>, Query, description = "Only include samples observed at/after (RFC 3339)"),
        ("to" = Option<String>, Query, description = "Only include samples observed at/before (RFC 3339)"),
        ("include_outliers" = Option<bool>, Query, description = "Include out-of-bounds samples in the estimate's input (default true)"),
        ("fallback_tier" = Option<String>, Query, description = "Only combos where pushback or taxi resolved at this ladder tier"),
        ("page" = Option<i64>, Query, description = "1-based page (default 1)"),
        ("page_size" = Option<i64>, Query, description = "Page size (default 50, max 100)")
    ),
    responses((status = 200, body = TaxiEstimatePage), (status = 400), (status = 401))
)]
pub async fn list_taxi_estimates(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsRead>,
    Query(q): Query<EstimatesQuery>,
) -> Result<Json<TaxiEstimatePage>, ApiError> {
    let airport = norm_opt(Some(q.airport)).ok_or(ApiError::BadRequest)?;
    let p = pool(&state)?;
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 100);
    let filters = EstimateFilters {
        airport,
        gate_id: norm_opt(q.gate_id),
        aircraft: norm_opt(q.aircraft),
        runway: norm_opt(q.runway),
        from: q.from,
        to: q.to,
        include_outliers: q.include_outliers.unwrap_or(true),
        fallback_tier: q
            .fallback_tier
            .as_deref()
            .and_then(|s| EstimateTier::parse(&s.to_ascii_lowercase())),
        limit: page_size,
        offset: (page - 1) * page_size,
    };
    let (items, total) = repo::fetch_taxi_estimates(p, &filters).await?;
    Ok(Json(TaxiEstimatePage {
        items,
        total,
        page,
        page_size,
    }))
}
