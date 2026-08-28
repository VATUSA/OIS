//! Dashboards: multiple named boards per user, collections, share-by-slug, and copy-a-shared-board.
//! All routes are gated by `AuthProfileRead` (any signed-in user) and are owner-scoped in the repo
//! WHERE clause — the owner id comes from the session, never the request. The shared-view route is
//! also signed-in-only, so a viewer's own session powers the widgets' live data. See migration 0035.

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::{
    auth::{
        context::CurrentUser, permissions::AuthProfileRead, require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        CopyResponse, CreateDashboardRequest, DashboardBody, DashboardCollection, DashboardLibrary,
        NameRequest, ShareResponse, SharedDashboardBody, UpdateDashboardRequest,
    },
    repos,
    state::AppState,
};

/// Defensive cap on a stored board blob (widgets + layout are client-owned, opaque).
const MAX_DASHBOARD_BYTES: usize = 512 * 1024;

fn ctx<'a>(
    state: &'a AppState,
    user: &'a Option<CurrentUser>,
) -> Result<(&'a sqlx::PgPool, &'a CurrentUser), ApiError> {
    let user = user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok((pool, user))
}

fn check_size(data: &Option<serde_json::Value>) -> Result<(), ApiError> {
    if let Some(v) = data
        && serde_json::to_vec(v).map_err(|_| ApiError::Internal)?.len() > MAX_DASHBOARD_BYTES
    {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

#[utoipa::path(
    get, path = "/api/v1/dashboards", tag = "dashboards",
    responses((status = 200, body = DashboardLibrary), (status = 401))
)]
pub async fn list_dashboards(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
) -> Result<Json<DashboardLibrary>, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    // Lazy one-time migration: seed a board from the legacy single-dashboard prefs blob.
    if repos::dashboards::count_dashboards(pool, &user.id).await? == 0
        && let Some(legacy) = repos::preferences::get_pref(pool, &user.id, "dashboard").await?
    {
        let has_widgets = legacy
            .get("widgets")
            .and_then(|w| w.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_widgets {
            repos::dashboards::create_dashboard(
                pool,
                &user.id,
                "My dashboard",
                Some(&legacy),
                None,
            )
            .await?;
        }
    }
    let dashboards = repos::dashboards::list_dashboards(pool, &user.id).await?;
    let collections = repos::dashboards::list_collections(pool, &user.id).await?;
    Ok(Json(DashboardLibrary {
        dashboards,
        collections,
    }))
}

#[utoipa::path(
    post, path = "/api/v1/dashboards", tag = "dashboards",
    request_body = CreateDashboardRequest,
    responses((status = 200, body = DashboardBody), (status = 400), (status = 401))
)]
pub async fn create_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Json(req): Json<CreateDashboardRequest>,
) -> Result<Json<DashboardBody>, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    check_size(&req.data)?;
    let board = repos::dashboards::create_dashboard(
        pool,
        &user.id,
        req.name.trim(),
        req.data.as_ref(),
        req.collection_id.as_deref(),
    )
    .await?;
    Ok(Json(board))
}

#[utoipa::path(
    get, path = "/api/v1/dashboards/{id}", tag = "dashboards",
    params(("id" = String, Path)),
    responses((status = 200, body = DashboardBody), (status = 401), (status = 404))
)]
pub async fn get_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<DashboardBody>, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    repos::dashboards::get_dashboard(pool, &user.id, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    put, path = "/api/v1/dashboards/{id}", tag = "dashboards",
    params(("id" = String, Path)), request_body = UpdateDashboardRequest,
    responses((status = 200, body = DashboardBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateDashboardRequest>,
) -> Result<Json<DashboardBody>, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    check_size(&req.data)?;
    repos::dashboards::update_dashboard(
        pool,
        &user.id,
        &id,
        req.name.as_deref().map(str::trim),
        req.data.as_ref(),
        req.collection_id.as_deref(),
    )
    .await?
    .map(Json)
    .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    delete, path = "/api/v1/dashboards/{id}", tag = "dashboards",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    if repos::dashboards::delete_dashboard(pool, &user.id, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(
    post, path = "/api/v1/dashboards/{id}/share", tag = "dashboards",
    params(("id" = String, Path)),
    responses((status = 200, body = ShareResponse), (status = 401), (status = 404))
)]
pub async fn share_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<ShareResponse>, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    let slug = Uuid::new_v4().simple().to_string();
    let effective = repos::dashboards::set_share(pool, &user.id, &id, &slug)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(ShareResponse {
        share_slug: effective,
    }))
}

#[utoipa::path(
    delete, path = "/api/v1/dashboards/{id}/share", tag = "dashboards",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn unshare_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    if repos::dashboards::clear_share(pool, &user.id, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(
    get, path = "/api/v1/dashboards/shared/{slug}", tag = "dashboards",
    params(("slug" = String, Path)),
    responses((status = 200, body = SharedDashboardBody), (status = 401), (status = 404))
)]
pub async fn get_shared_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(slug): Path<String>,
) -> Result<Json<SharedDashboardBody>, ApiError> {
    let (pool, _user) = ctx(&state, &user)?;
    repos::dashboards::get_shared(pool, &slug)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/dashboards/shared/{slug}/copy", tag = "dashboards",
    params(("slug" = String, Path)),
    responses((status = 200, body = CopyResponse), (status = 401), (status = 404))
)]
pub async fn copy_shared_dashboard(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(slug): Path<String>,
) -> Result<Json<CopyResponse>, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    let id = repos::dashboards::copy_shared(pool, &slug, &user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(CopyResponse { id }))
}

#[utoipa::path(
    post, path = "/api/v1/dashboard-collections", tag = "dashboards",
    request_body = NameRequest,
    responses((status = 200, body = DashboardCollection), (status = 401))
)]
pub async fn create_collection(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Json(req): Json<NameRequest>,
) -> Result<Json<DashboardCollection>, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    let c = repos::dashboards::create_collection(pool, &user.id, req.name.trim()).await?;
    Ok(Json(c))
}

#[utoipa::path(
    put, path = "/api/v1/dashboard-collections/{id}", tag = "dashboards",
    params(("id" = String, Path)), request_body = NameRequest,
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn rename_collection(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(req): Json<NameRequest>,
) -> Result<StatusCode, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    if repos::dashboards::rename_collection(pool, &user.id, &id, req.name.trim()).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(
    delete, path = "/api/v1/dashboard-collections/{id}", tag = "dashboards",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_collection(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (pool, user) = ctx(&state, &user)?;
    if repos::dashboards::delete_collection(pool, &user.id, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
