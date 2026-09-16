//! The Admin page's landing summary (#292).

use axum::{Extension, Json, extract::State};

use crate::{
    auth::{
        acl::{self, PermissionPath},
        context::{CurrentApiKey, CurrentUser},
        permissions::{AccessUsersRead, AuditLogsRead, SystemJobsRead},
        principal::Principal,
        require_permission::Permission,
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

/// Each section is included only if the principal effectively holds that page's read permission —
/// the same deny-aware, key-capped set `RequirePermission` checks, so the summary never shows what the
/// page it summarises would refuse.
async fn build_summary(
    state: &AppState,
    principal: &Principal,
) -> Result<AdminSummaryBody, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let (_, effective) = match principal {
        Principal::User(u) => acl::fetch_user_access(Some(pool), &u.id).await?,
        Principal::ApiKey(k) => acl::fetch_api_key_access(Some(pool), k).await?,
    };
    let holds = |permission: PermissionPath| effective.contains(&permission);

    let audit_events = if holds(AuditLogsRead::path()) {
        Some(admin_repo::daily_counts(pool, CountedTable::AuditLogs, SUMMARY_DAYS).await?)
    } else {
        None
    };
    let new_users = if holds(AccessUsersRead::path()) {
        Some(admin_repo::daily_counts(pool, CountedTable::Users, SUMMARY_DAYS).await?)
    } else {
        None
    };
    let jobs = if holds(SystemJobsRead::path()) {
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

    /// A role grants all three reads, but explicit denies override them — as `RequirePermission`
    /// would refuse the pages, the summary must withhold every section.
    #[sqlx::test]
    async fn explicit_denies_withhold_role_granted_sections(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant_via_role_then_deny(&pool, &user).await;
        let state = test_state(pool, HashMap::new());

        let body = build_summary(&state, &principal_for(&user)).await.unwrap();

        assert!(body.audit_events.is_none());
        assert!(body.new_users.is_none());
        assert!(body.jobs.is_none());
    }

    #[sqlx::test]
    async fn an_api_key_gets_no_section_its_owner_is_denied(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant_via_role_then_deny(&pool, &user).await;
        let key_id = crate::repos::api_keys::create_api_key(
            &pool,
            &user,
            "summary-test",
            None,
            "ois_pat_test",
            "summary-test-hash",
            None,
            &[("audit.logs.read".to_string(), None)],
        )
        .await
        .unwrap();
        let state = test_state(pool, HashMap::new());
        let key = Principal::ApiKey(CurrentApiKey {
            id: key_id,
            owner_user_id: user,
            prefix: "ois_pat_test".to_string(),
            name: "summary-test".to_string(),
        });

        let body = build_summary(&state, &key).await.unwrap();

        assert!(body.audit_events.is_none());
    }

    #[sqlx::test]
    async fn an_api_key_gets_the_sections_it_was_granted(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "audit.logs.read", None).await;
        grant(&pool, &user, "system.jobs.read", None).await;
        let key_id = crate::repos::api_keys::create_api_key(
            &pool,
            &user,
            "summary-test",
            None,
            "ois_pat_test",
            "summary-test-hash",
            None,
            &[("audit.logs.read".to_string(), None)],
        )
        .await
        .unwrap();
        let state = test_state(pool, HashMap::new());
        let key = Principal::ApiKey(CurrentApiKey {
            id: key_id,
            owner_user_id: user,
            prefix: "ois_pat_test".to_string(),
            name: "summary-test".to_string(),
        });

        let body = build_summary(&state, &key).await.unwrap();

        assert!(body.audit_events.is_some());
        assert!(body.jobs.is_none());
    }

    async fn grant_via_role_then_deny(pool: &PgPool, user: &str) {
        for sql in [
            "insert into access.roles (name, description, is_system) values ('SUMMARY_TEST', 'test', false)",
            "insert into access.role_permissions (role_name, permission_name) values \
             ('SUMMARY_TEST', 'audit.logs.read'), ('SUMMARY_TEST', 'access.users.read'), \
             ('SUMMARY_TEST', 'system.jobs.read')",
        ] {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
        sqlx::query(
            "insert into access.user_roles (user_id, role_name) values ($1, 'SUMMARY_TEST')",
        )
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted) \
             select $1, name, false from access.permissions \
             where name in ('audit.logs.read', 'access.users.read', 'system.jobs.read')",
        )
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
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
