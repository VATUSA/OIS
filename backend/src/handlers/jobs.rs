//! Admin background-tasks viewer (issue #34): list every background job's last-run status and
//! trigger a triggerable one to run now. Reads need `system.jobs.read`; triggering needs
//! `system.jobs.update`.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::{
    auth::{
        permissions::{SystemJobsRead, SystemJobsUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    job_registry::JobStatus,
    state::AppState,
};

#[utoipa::path(
    get, path = "/api/v1/admin/jobs", tag = "system",
    responses((status = 200, body = Vec<JobStatus>), (status = 401))
)]
pub async fn list_jobs(
    State(state): State<AppState>,
    _permission: RequirePermission<SystemJobsRead>,
) -> Json<Vec<JobStatus>> {
    Json(state.jobs.snapshot())
}

#[utoipa::path(
    post, path = "/api/v1/admin/jobs/{name}/run", tag = "system",
    params(("name" = String, Path, description = "Job name")),
    responses((status = 202), (status = 401), (status = 404))
)]
pub async fn run_job(
    State(state): State<AppState>,
    _permission: RequirePermission<SystemJobsUpdate>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    // 404 covers both an unknown job and one that isn't triggerable (e.g. a continuous poller).
    if state.jobs.trigger(&name) {
        Ok(StatusCode::ACCEPTED)
    } else {
        Err(ApiError::NotFound)
    }
}
