//! The Admin page's landing summary (#292).

use axum::{Extension, Json, extract::State};

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        principal::Principal,
    },
    errors::ApiError,
    models::{AdminSummaryBody, JobsHealth},
    repos::admin::{self as admin_repo, CountedTable, SUMMARY_DAYS},
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/api/v1/admin/summary",
    tag = "system",
    responses((status = 200, body = AdminSummaryBody), (status = 401))
)]
pub async fn get_admin_summary(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
) -> Result<Json<AdminSummaryBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    Ok(Json(build_summary(&state, &principal).await?))
}

/// Each section is included only if the principal holds that page's read permission.
async fn build_summary(
    state: &AppState,
    principal: &Principal,
) -> Result<AdminSummaryBody, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let holds = async |permission: &str| -> Result<bool, ApiError> {
        Ok(!principal
            .permission_scope(state, permission)
            .await?
            .is_empty())
    };

    let audit_events = if holds("audit.logs.read").await? {
        Some(admin_repo::daily_counts(pool, CountedTable::AuditLogs, SUMMARY_DAYS).await?)
    } else {
        None
    };
    let new_users = if holds("access.users.read").await? {
        Some(admin_repo::daily_counts(pool, CountedTable::Users, SUMMARY_DAYS).await?)
    } else {
        None
    };
    let jobs = if holds("system.jobs.read").await? {
        let snapshot = state.jobs.snapshot();
        Some(JobsHealth {
            total: snapshot.len() as i64,
            failing: snapshot.iter().filter(|j| j.last_ok == Some(false)).count() as i64,
        })
    } else {
        None
    };

    Ok(AdminSummaryBody {
        audit_events,
        new_users,
        jobs,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sqlx::PgPool;

    use super::*;
    use crate::scope_test_support::{grant, principal_for, seed_user, test_state};

    #[sqlx::test]
    async fn sections_follow_the_callers_permissions(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "audit.logs.read", None).await;
        let state = test_state(pool, HashMap::new());

        let body = build_summary(&state, &principal_for(&user)).await.unwrap();

        assert!(body.audit_events.is_some());
        assert!(body.new_users.is_none());
        assert!(body.jobs.is_none());
    }

    #[sqlx::test]
    async fn no_permissions_yields_an_empty_summary(pool: PgPool) {
        let user = seed_user(&pool).await;
        let state = test_state(pool, HashMap::new());

        let body = build_summary(&state, &principal_for(&user)).await.unwrap();

        assert!(body.audit_events.is_none() && body.new_users.is_none() && body.jobs.is_none());
    }

    #[sqlx::test]
    async fn jobs_health_counts_failed_last_runs(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "system.jobs.read", None).await;
        let state = test_state(pool, HashMap::new());
        state.jobs.register("ok", "", None, false);
        state.jobs.register("bad", "", None, false);
        state.jobs.register("never", "", None, false);
        state.jobs.finish("ok", true, "");
        state.jobs.finish("bad", false, "boom");

        let jobs = build_summary(&state, &principal_for(&user))
            .await
            .unwrap()
            .jobs
            .unwrap();

        assert_eq!((jobs.total, jobs.failing), (3, 1));
    }
}
