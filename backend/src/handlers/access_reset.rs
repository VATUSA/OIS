//! Reset every member's access to exactly what VATUSA justifies (#795): a dry run, and the reset itself,
//! which runs in the background and is polled for its result (#806).
//! Server admin only. The gate is the `SERVER_ADMIN` group, not a catalog permission, so no group can
//! be given it.

use std::future::Future;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    auth::{
        acl::{fetch_user_access, is_server_admin},
        context::CurrentUser,
    },
    errors::ApiError,
    models::{
        AccessResetBody, AccessResetFailure, AccessResetGrant, AccessResetRequest, AccessResetRun,
        AccessResetStarted, AccessResetUser,
    },
    repos::{
        access_reset as reset_runs, audit as audit_repo,
        vatusa::{self as vatusa_repo, GrantRow, ResetMode, ResetRun},
    },
    state::AppState,
};

/// Why the reset endpoint refused or a run stopped: an ordinary API error, a failure the admin is shown
/// with its cause and how many members were already reset, or another run in progress.
pub enum ResetError {
    Api(ApiError),
    Failed(StatusCode, AccessResetFailure),
    /// Another reset is running: 409 with its run id.
    Running(String),
}

impl From<ApiError> for ResetError {
    fn from(e: ApiError) -> Self {
        Self::Api(e)
    }
}

impl IntoResponse for ResetError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(e) => e.into_response(),
            Self::Failed(status, body) => (status, Json(body)).into_response(),
            Self::Running(run_id) => {
                (StatusCode::CONFLICT, Json(AccessResetStarted { run_id })).into_response()
            }
        }
    }
}

async fn require_server_admin(state: &AppState, user: &CurrentUser) -> Result<(), ApiError> {
    let (roles, _) = fetch_user_access(state.db.as_ref(), &user.id).await?;
    if is_server_admin(&roles) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn grant_body(row: GrantRow) -> AccessResetGrant {
    AccessResetGrant {
        kind: if row.is_group { "group" } else { "permission" }.to_string(),
        name: row.name,
        artcc_id: row.artcc_id,
        source: row.source,
        granted: row.granted,
    }
}

fn reset_body(dry_run: bool, pull_summary: Option<String>, run: ResetRun) -> AccessResetBody {
    AccessResetBody {
        dry_run,
        pull_summary,
        users_checked: run.users_checked as i64,
        users_reset: run.changed.len() as i64,
        users: run
            .changed
            .into_iter()
            .map(|m| AccessResetUser {
                cid: m.cid,
                display_name: m.display_name,
                reattached: m.reattached,
                added: m.added.into_iter().map(grant_body).collect(),
                removed: m.removed.into_iter().map(grant_body).collect(),
            })
            .collect(),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/access/vatusa-reset",
    tag = "access",
    responses((status = 200, body = AccessResetBody), (status = 401), (status = 403)),
    security(("session" = []))
)]
/// Dry run of the reset (#795): every member whose access would change, and the grant rows each would
/// gain and lose, from the VATUSA data the last division pull stored. Writes nothing. Server admin only.
pub async fn preview_vatusa_reset(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<Json<AccessResetBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    require_server_admin(&state, user).await?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut run = vatusa_repo::reset_all(pool, &ResetMode::DryRun).await;
    if let Some(e) = run.failure.take() {
        return Err(e);
    }
    Ok(Json(reset_body(true, None, run)))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/access/vatusa-reset",
    tag = "access",
    request_body = AccessResetRequest,
    responses(
        (status = 202, description = "The reset started; poll its run for the result", body = AccessResetStarted),
        (status = 400),
        (status = 401),
        (status = 403),
        (status = 409, description = "A reset is already running; poll that run instead", body = AccessResetStarted),
        (status = 503, description = "VATUSA is not configured; nothing was started", body = AccessResetFailure)
    ),
    security(("session" = []))
)]
/// Start a reset of every member's access to VATUSA (#795) and answer at once with its run id (#806).
/// The run pulls the division fresh, then, one transaction per member, puts them back on role sync,
/// deletes every hand-made grant except the baseline `USER` and `SERVER_ADMIN` groups, and reconciles
/// their VATUSA grants. `system` grants are left alone. Each changed member gets one `USER_ACCESS`
/// audit entry with the reason. Because the pull is fresh, the result can differ from the dry run if
/// VATUSA changed since the last pull. It runs in the background, so it finishes whether or not the
/// caller waits; `GET /api/v1/admin/access/vatusa-reset/runs/{id}` returns its result. One reset runs at
/// a time, never alongside the division pull. Server admin only.
pub async fn apply_vatusa_reset(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Json(payload): Json<AccessResetRequest>,
) -> Result<(StatusCode, Json<AccessResetStarted>), ResetError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    require_server_admin(&state, user).await?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    // The reason is checked before the key, so a blank one is a 400 wherever VATUSA is configured.
    if payload.reason.trim().is_empty() {
        return Err(ApiError::BadRequest.into());
    }
    let Some(api_key) = crate::config::vatusa_api_key() else {
        return Err(pull_failed(PullError::NotConfigured));
    };
    let pull = division_pull(pool.clone(), state.events.clone(), api_key);
    let run_id = start_reset(
        &state,
        user,
        &payload.reason,
        audit_repo::client_ip(&headers),
        pull,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(AccessResetStarted { run_id })))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/access/vatusa-reset/runs/{id}",
    tag = "access",
    params(("id" = String, Path, description = "The run id the reset's POST returned")),
    responses((status = 200, body = AccessResetRun), (status = 401), (status = 403), (status = 404)),
    security(("session" = []))
)]
/// One reset run (#806): `running` until it finishes, then its result or why it failed. A run whose
/// backend stopped before it finished is reported as failed (`reset_interrupted`). Server admin only.
pub async fn get_vatusa_reset_run(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<AccessResetRun>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    require_server_admin(&state, user).await?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::NotFound)?;
    reset_runs::fetch_run(pool, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Why the division pull that opens a reset did not run.
pub enum PullError {
    NotConfigured,
    Failed(String),
}

/// The response for a pull that did not run: nothing was reset.
fn pull_failed(e: PullError) -> ResetError {
    let (status, error, message) = match e {
        PullError::NotConfigured => (
            StatusCode::SERVICE_UNAVAILABLE,
            "vatusa_not_configured",
            "VATUSA is not configured (VATUSA_API_KEY is unset), so there is nothing to reset to"
                .to_string(),
        ),
        PullError::Failed(message) => (StatusCode::BAD_GATEWAY, "vatusa_pull_failed", message),
    };
    ResetError::Failed(
        status,
        AccessResetFailure {
            error: error.to_string(),
            message,
            users_reset: 0,
        },
    )
}

/// The division pull a reset opens with. It owns what it needs, since it runs after the request.
async fn division_pull(
    pool: sqlx::PgPool,
    events: crate::realtime::Events,
    api_key: String,
) -> Result<String, PullError> {
    crate::feed::vatusa::pull_division(&pool, &api_key, &events)
        .await
        .map_err(PullError::Failed)
}

/// The reset's entry in the job registry, so a run shows in Background Tasks and `/metrics`.
pub const RESET_JOB: &str = "vatusa_access_reset";

/// Start a reset in a task of its own and return its run id at once (#806): the run belongs to no
/// request, so a client or ingress that gives up cannot stop it part-way. The task holds the reset lock
/// for the whole run and the division lock from before its pull until its last member, so no other
/// reset, and no division pull, runs beside it on any replica. With the pull passed in so a test can
/// stand in for VATUSA.
async fn start_reset(
    state: &AppState,
    user: &CurrentUser,
    reason: &str,
    ip_address: Option<String>,
    pull: impl Future<Output = Result<String, PullError>> + Send + 'static,
) -> Result<String, ResetError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(ApiError::BadRequest.into());
    }
    let Some(mut locks) = reset_runs::try_lock_reset(pool)
        .await
        .map_err(|_| ApiError::Internal)?
    else {
        return match reset_runs::running_run(pool).await? {
            Some(run_id) => Err(ResetError::Running(run_id.to_string())),
            None => Err(ApiError::Conflict.into()),
        };
    };
    let run_id = reset_runs::start_run(pool, &user.id, reason).await?;
    state.jobs.register(
        RESET_JOB,
        "Reset every member's access to VATUSA: pull the division, then reset each member (started by a server admin)",
        None,
        false,
    );
    state.jobs.begin(RESET_JOB);

    let (state, user, reason) = (state.clone(), user.clone(), reason.to_string());
    tokio::spawn(async move {
        let outcome = match locks.lock_division().await {
            Ok(()) => reset_to_vatusa(&state, &user, &reason, ip_address, pull).await,
            Err(e) => {
                tracing::error!(error = %e, "access reset to VATUSA could not take the division lock");
                Err(ApiError::Internal.into())
            }
        };
        let outcome = outcome.map_err(|e| match e {
            ResetError::Failed(_, failure) => failure,
            ResetError::Api(e) => AccessResetFailure {
                error: "reset_failed".to_string(),
                message: format!("the reset could not run ({e}); nothing was reset"),
                users_reset: 0,
            },
            ResetError::Running(_) => unreachable!("only start_reset refuses a second run"),
        });
        // Stored while the locks are still held: a `running` row with no lock holder reads as
        // interrupted.
        let pool = state.db.as_ref().expect("start_reset checked the pool");
        if let Err(e) = reset_runs::finish_run(pool, run_id, outcome.as_ref()).await {
            tracing::error!(error = %e, %run_id, "access reset to VATUSA finished but its result was not stored");
        }
        match &outcome {
            Ok(body) => state.jobs.finish(
                RESET_JOB,
                true,
                format!(
                    "{} of {} members reset",
                    body.users_reset, body.users_checked
                ),
            ),
            Err(failure) => state.jobs.finish(RESET_JOB, false, failure.message.clone()),
        }
        drop(locks);
    });
    Ok(run_id.to_string())
}

/// The reset itself, run by [`start_reset`]'s task. The pull runs first; if it fails, nothing is reset.
async fn reset_to_vatusa(
    state: &AppState,
    user: &CurrentUser,
    reason: &str,
    ip_address: Option<String>,
    pull: impl Future<Output = Result<String, PullError>>,
) -> Result<AccessResetBody, ResetError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(ApiError::BadRequest.into());
    }
    let pull_summary = pull.await.map_err(pull_failed)?;

    let mut run = vatusa_repo::reset_all(
        pool,
        &ResetMode::Apply {
            actor_id: audit_repo::fetch_user_actor_id(pool, &user.id).await?,
            reason,
            ip_address,
        },
    )
    .await;
    if !run.changed.is_empty() {
        state.publish(crate::realtime::topic::ACCESS_GRANTED);
    }
    if let Some(e) = run.failure.take() {
        tracing::error!(error = %e, users_reset = run.changed.len(), "access reset to VATUSA stopped part-way");
        return Err(ResetError::Failed(
            StatusCode::INTERNAL_SERVER_ERROR,
            AccessResetFailure {
                error: "reset_incomplete".to_string(),
                message: format!(
                    "the reset stopped part-way ({e}); the members already reset stay reset, the rest \
                     are untouched — run it again to finish"
                ),
                users_reset: run.changed.len() as i64,
            },
        ));
    }
    Ok(reset_body(false, Some(pull_summary), run))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sqlx::PgPool;

    use super::*;
    use crate::scope_test_support::{grant, send, send_json, session_cookie, test_state};

    const ADMIN_CID: i64 = 1_795_000;
    const MEMBER_CID: i64 = 1_795_001;
    const STEADY_CID: i64 = 1_795_002;

    /// `(kind, name, scope, source, granted)`, sorted, for one user.
    type Rows = Vec<(String, String, Option<String>, String, bool)>;

    struct World {
        pool: PgPool,
        state: AppState,
        admin: String,
        admin_cookie: String,
        /// Holds one of every kind of row the reset must remove, keep, or add.
        member: String,
        /// Already exactly what VATUSA justifies: the reset leaves them alone.
        steady: String,
    }

    async fn user(pool: &PgPool, cid: i64, name: &str) -> String {
        sqlx::query_scalar(
            "insert into identity.users (full_name, display_name, cid, vatusa_synced_at) \
             values ($2, $2, $1, now()) returning id",
        )
        .bind(cid)
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn group(pool: &PgPool, user: &str, role: &str, artcc: Option<&str>, source: &str) {
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, artcc_id, source) \
             values ($1, $2, $3, $4)",
        )
        .bind(user)
        .bind(role)
        .bind(artcc)
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn permission(
        pool: &PgPool,
        user: &str,
        name: &str,
        artcc: Option<&str>,
        source: &str,
        granted: bool,
    ) {
        sqlx::query(
            "insert into access.user_permissions \
             (user_id, permission_name, artcc_id, source, granted) values ($1, $2, $3, $4, $5)",
        )
        .bind(user)
        .bind(name)
        .bind(artcc)
        .bind(source)
        .bind(granted)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn vatusa_role(pool: &PgPool, cid: i64, facility: &str, role: &str) {
        sqlx::query("insert into identity.vatusa_roles (cid, facility, role) values ($1, $2, $3)")
            .bind(cid)
            .bind(facility)
            .bind(role)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn detach(pool: &PgPool, user: &str) {
        sqlx::query("update identity.users set vatusa_roles_detached_at = now() where id = $1")
            .bind(user)
            .execute(pool)
            .await
            .unwrap();
    }

    /// The server admin holds `USER` and `SERVER_ADMIN` as `manual`, as migration 0098 backfilled them,
    /// plus a hand-made EC. The member holds every kind of row: the baseline as `manual` and as
    /// `system`; a `system` non-baseline group and a `system` permission (kept — the reset only takes
    /// `manual` rows); a hand-made group, permission and deny (removed); VATUSA's EC at ZDC, which the
    /// default EVENT_COORDINATOR mapping (0124) still justifies (kept); and VATUSA's AEC at ZDC, whose
    /// role is gone (removed). They are detached, and VATUSA justifies an EC at ZJX they don't hold yet.
    async fn world(pool: PgPool) -> World {
        let admin = user(&pool, ADMIN_CID, "Admin").await;
        group(&pool, &admin, "USER", None, "manual").await;
        group(&pool, &admin, "SERVER_ADMIN", None, "manual").await;
        group(&pool, &admin, "EC", None, "manual").await;
        let admin_cookie = session_cookie(&pool, &admin).await;
        // Sign-in creates the audit actor; the reset is attributed to it.
        let mut tx = pool.begin().await.unwrap();
        crate::repos::access::ensure_user_actor(&mut tx, &admin, "Admin")
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let member = user(&pool, MEMBER_CID, "Member").await;
        vatusa_role(&pool, MEMBER_CID, "ZDC", "EVENT_COORDINATOR").await;
        vatusa_role(&pool, MEMBER_CID, "ZJX", "EVENT_COORDINATOR").await;
        group(&pool, &member, "USER", None, "manual").await;
        group(&pool, &member, "USER", None, "system").await;
        group(&pool, &member, "CONTROLLER", Some("ZDC"), "system").await;
        group(&pool, &member, "ACE", Some("ZDC"), "manual").await;
        group(&pool, &member, "EC", Some("ZDC"), "vatusa").await;
        group(&pool, &member, "AEC", Some("ZDC"), "vatusa").await;
        permission(
            &pool,
            &member,
            "access.users.read",
            Some("ZDC"),
            "manual",
            true,
        )
        .await;
        permission(&pool, &member, "tmu.tmi.publish", None, "manual", false).await;
        permission(&pool, &member, "access.catalog.read", None, "system", true).await;
        detach(&pool, &member).await;

        let steady = user(&pool, STEADY_CID, "Steady").await;
        vatusa_role(&pool, STEADY_CID, "ZDC", "EVENT_COORDINATOR").await;
        group(&pool, &steady, "USER", None, "system").await;
        group(&pool, &steady, "EC", Some("ZDC"), "vatusa").await;

        World {
            state: test_state(pool.clone(), Default::default()),
            pool,
            admin,
            admin_cookie,
            member,
            steady,
        }
    }

    async fn rows(pool: &PgPool, user: &str) -> Rows {
        sqlx::query_as(
            "select 'group', role_name, artcc_id, source, true from access.user_roles \
             where user_id = $1 \
             union all \
             select 'permission', permission_name, artcc_id, source, granted \
             from access.user_permissions where user_id = $1 \
             order by 1, 2, 3 nulls first, 4",
        )
        .bind(user)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    fn row(kind: &str, name: &str, artcc: Option<&str>, source: &str, granted: bool) -> Rows {
        vec![(
            kind.into(),
            name.into(),
            artcc.map(Into::into),
            source.into(),
            granted,
        )]
    }

    fn rows_of(parts: &[Rows]) -> Rows {
        let mut all: Rows = parts.concat();
        all.sort_by(|a, b| {
            (&a.0, &a.1, a.2.is_some(), &a.2, &a.3).cmp(&(&b.0, &b.1, b.2.is_some(), &b.2, &b.3))
        });
        all
    }

    async fn is_detached(pool: &PgPool, user: &str) -> bool {
        sqlx::query_scalar(
            "select vatusa_roles_detached_at is not null from identity.users where id = $1",
        )
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Every `USER_ACCESS` audit reason recorded for `user`, oldest first.
    async fn audits(pool: &PgPool, user: &str) -> Vec<(Option<String>, String, bool)> {
        sqlx::query_as(
            "select actor_id, reason, before_state is not null and after_state is not null \
             from access.audit_logs where resource_type = 'USER_ACCESS' and resource_id = $1 \
             order by created_at",
        )
        .bind(user)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Everything the reset could write, for asserting that something wrote nothing.
    async fn everything(pool: &PgPool) -> (Rows, Vec<(String, bool)>, i64, i64) {
        let users: Vec<String> = sqlx::query_scalar("select id from identity.users order by id")
            .fetch_all(pool)
            .await
            .unwrap();
        let mut all = Vec::new();
        let mut detached = Vec::new();
        for u in &users {
            all.extend(rows(pool, u).await);
            detached.push((u.clone(), is_detached(pool, u).await));
        }
        let audits: i64 = sqlx::query_scalar("select count(*) from access.audit_logs")
            .fetch_one(pool)
            .await
            .unwrap();
        let service_roles: i64 =
            sqlx::query_scalar("select count(*) from access.service_account_roles")
                .fetch_one(pool)
                .await
                .unwrap();
        (all, detached, audits, service_roles)
    }

    /// The member's latest `USER_ACCESS` audit snapshots, `(before, after)`.
    async fn snapshot(pool: &PgPool, user: &str) -> (serde_json::Value, serde_json::Value) {
        let (before, after): (String, String) = sqlx::query_as(
            "select before_state::text, after_state::text from access.audit_logs \
             where resource_type = 'USER_ACCESS' and resource_id = $1 \
             order by created_at desc limit 1",
        )
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap();
        (
            serde_json::from_str(&before).unwrap(),
            serde_json::from_str(&after).unwrap(),
        )
    }

    /// The group names a snapshot lists at one scope (`None` = national); empty when the scope is
    /// absent.
    fn scope_roles(snapshot: &serde_json::Value, artcc: Option<&str>) -> Vec<String> {
        let mut roles: Vec<String> = snapshot["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["artcc_id"].as_str() == artcc)
            .flat_map(|s| s["role_names"].as_array().unwrap().iter())
            .map(|r| r.as_str().unwrap().to_string())
            .collect();
        roles.sort();
        roles
    }

    fn admin_user(w: &World) -> CurrentUser {
        CurrentUser {
            id: w.admin.clone(),
            cid: ADMIN_CID,
            email: String::new(),
            display_name: "Admin".into(),
            rating: None,
            primary_role: None,
        }
    }

    async fn pulled() -> Result<String, PullError> {
        Ok("pulled".to_string())
    }

    async fn reset(w: &World) -> Result<AccessResetBody, ResetError> {
        reset_to_vatusa(&w.state, &admin_user(w), "back to VATUSA", None, pulled()).await
    }

    /// What the member holds after a reset: their `system` rows, the baseline they held as `manual`,
    /// and exactly the EC grants VATUSA justifies.
    fn member_after() -> Rows {
        rows_of(&[
            row("group", "CONTROLLER", Some("ZDC"), "system", true),
            row("group", "EC", Some("ZDC"), "vatusa", true),
            row("group", "EC", Some("ZJX"), "vatusa", true),
            row("group", "USER", None, "manual", true),
            row("group", "USER", None, "system", true),
            row("permission", "access.catalog.read", None, "system", true),
        ])
    }

    /// AC3 + AC4: after a reset the member holds exactly their `system` grants plus what VATUSA
    /// justifies; no hand-made row is left but the protected baseline; they are back on role sync. The
    /// server admin keeps `USER` and `SERVER_ADMIN` though both are stored as `manual`, and loses the
    /// hand-made EC. Service accounts are untouched.
    #[sqlx::test]
    async fn a_reset_leaves_exactly_system_and_vatusa_grants(pool: PgPool) {
        let w = world(pool).await;
        let (_, _, _, service_roles_before) = everything(&w.pool).await;

        let mut nudges = w.state.events.subscribe();
        let body = reset(&w).await.ok().unwrap();

        let nudge = nudges
            .try_recv()
            .expect("the reset tells signed-in browsers");
        assert_eq!(nudge.topic, crate::realtime::topic::ACCESS_GRANTED);
        assert!(reset(&w).await.ok().unwrap().users.is_empty());
        assert!(
            nudges.try_recv().is_err(),
            "a reset that changes nothing tells no one"
        );
        assert_eq!(rows(&w.pool, &w.member).await, member_after());
        assert!(!is_detached(&w.pool, &w.member).await);
        assert_eq!(
            rows(&w.pool, &w.admin).await,
            rows_of(&[
                row("group", "SERVER_ADMIN", None, "manual", true),
                row("group", "USER", None, "manual", true),
            ])
        );
        assert_eq!(
            rows(&w.pool, &w.steady).await,
            rows_of(&[
                row("group", "EC", Some("ZDC"), "vatusa", true),
                row("group", "USER", None, "system", true),
            ])
        );
        let manual_left: Vec<String> = sqlx::query_scalar(
            "select role_name from access.user_roles where source = 'manual' \
             union all select permission_name from access.user_permissions where source = 'manual' \
             order by 1",
        )
        .fetch_all(&w.pool)
        .await
        .unwrap();
        assert_eq!(manual_left, ["SERVER_ADMIN", "USER", "USER"]);
        assert_eq!(everything(&w.pool).await.3, service_roles_before);

        assert!(!body.dry_run);
        assert_eq!(body.pull_summary.as_deref(), Some("pulled"));
        assert_eq!(body.users_checked, 3);
        assert_eq!(
            body.users_reset, 2,
            "the admin and the member; not the steady one"
        );
    }

    /// AC6: one audit entry per changed member, by the admin, with both snapshots and the reason
    /// naming every row it removed and added. The unchanged member gets none.
    #[sqlx::test]
    async fn each_changed_member_gets_one_audit_entry(pool: PgPool) {
        let w = world(pool).await;
        let actor = audit_repo::fetch_user_actor_id(&w.pool, &w.admin)
            .await
            .unwrap();
        assert!(actor.is_some());
        reset(&w).await.ok().unwrap();

        let member = audits(&w.pool, &w.member).await;
        assert_eq!(member.len(), 1, "{member:?}");
        let (by, reason, snapshots) = &member[0];
        assert_eq!(by, &actor);
        assert!(snapshots);
        // The before-state is the undo trail: it must hold what the reset took away, and the
        // after-state must not.
        let (before, after) = snapshot(&w.pool, &w.member).await;
        assert_eq!(
            scope_roles(&before, Some("ZDC")),
            ["ACE", "AEC", "CONTROLLER", "EC"]
        );
        let zdc_before = before["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["artcc_id"] == "ZDC")
            .unwrap();
        assert_eq!(
            zdc_before["permissions"],
            json!({"access": {"users": ["read"]}})
        );
        assert_eq!(scope_roles(&after, Some("ZDC")), ["CONTROLLER", "EC"]);
        assert_eq!(scope_roles(&after, Some("ZJX")), ["EC"]);
        assert_eq!(scope_roles(&before, Some("ZJX")), Vec::<String>::new());
        assert!(
            reason.starts_with("Reset to VATUSA: back to VATUSA ("),
            "{reason}"
        );
        for change in [
            "removed group ACE at ZDC (manual)",
            "removed group AEC at ZDC (vatusa)",
            "removed permission access.users.read at ZDC (manual)",
            "removed deny tmu.tmi.publish nationally (manual)",
            "added group EC at ZJX (vatusa)",
            "re-attached to VATUSA role sync",
        ] {
            assert!(reason.contains(change), "{change} missing from {reason}");
        }
        assert_eq!(audits(&w.pool, &w.admin).await.len(), 1);
        assert_eq!(audits(&w.pool, &w.steady).await, []);
    }

    /// A member whose grants already match VATUSA but who was detached is still reset: re-attaching
    /// them is a change, and it is audited.
    #[sqlx::test]
    async fn re_attaching_alone_is_a_change(pool: PgPool) {
        let w = world(pool).await;
        detach(&w.pool, &w.steady).await;
        let body = reset(&w).await.ok().unwrap();
        assert!(!is_detached(&w.pool, &w.steady).await);
        let steady = body
            .users
            .iter()
            .find(|u| u.cid == Some(STEADY_CID))
            .expect("the re-attached member is listed");
        assert!(steady.reattached && steady.added.is_empty() && steady.removed.is_empty());
        assert_eq!(audits(&w.pool, &w.steady).await.len(), 1);
    }

    /// AC2: the dry run, through the real route, lists each member's added and removed rows, and writes
    /// nothing — no grant, audit, or detach change. Applying it then does exactly what it listed.
    #[sqlx::test]
    async fn the_dry_run_reports_and_writes_nothing(pool: PgPool) {
        let w = world(pool).await;
        let before = everything(&w.pool).await;

        let (status, preview) = send_json(
            &w.state,
            http::Method::GET,
            "/api/v1/admin/access/vatusa-reset",
            &w.admin_cookie,
        )
        .await;
        assert_eq!(status, http::StatusCode::OK, "{preview}");
        assert_eq!(
            everything(&w.pool).await,
            before,
            "a dry run wrote something"
        );

        assert_eq!(preview["dry_run"], true);
        assert_eq!(preview["users_reset"], 2);
        let member = preview["users"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["cid"] == MEMBER_CID)
            .unwrap();
        assert_eq!(member["reattached"], true);
        assert_eq!(
            member["added"],
            json!([{"kind": "group", "name": "EC", "artcc_id": "ZJX", "source": "vatusa", "granted": true}])
        );
        let removed: Vec<(String, String, String)> = member["removed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["name"].as_str().unwrap().to_string(),
                    r["source"].as_str().unwrap().to_string(),
                    r["granted"].to_string(),
                )
            })
            .collect();
        assert_eq!(
            removed,
            [
                ("access.users.read".into(), "manual".into(), "true".into()),
                ("tmu.tmi.publish".into(), "manual".into(), "false".into()),
                ("ACE".into(), "manual".into(), "true".into()),
                ("AEC".into(), "vatusa".into(), "true".into()),
            ]
        );

        let applied = reset(&w).await.ok().unwrap();
        assert_eq!(
            serde_json::to_value(&applied.users).unwrap(),
            preview["users"],
            "the reset does exactly what the dry run listed"
        );
    }

    /// AC5: a failed VATUSA pull resets no one and says why.
    #[sqlx::test]
    async fn a_failed_pull_changes_nothing(pool: PgPool) {
        let w = world(pool).await;
        let before = everything(&w.pool).await;

        let failed = reset_to_vatusa(&w.state, &admin_user(&w), "back to VATUSA", None, async {
            Err(PullError::Failed(
                "VATUSA division pull failed: 502 Bad Gateway".to_string(),
            ))
        })
        .await;
        let Err(ResetError::Failed(status, body)) = failed else {
            panic!("a failed pull must fail the reset");
        };
        assert_eq!(status, http::StatusCode::BAD_GATEWAY);
        assert_eq!(body.error, "vatusa_pull_failed");
        assert_eq!(body.message, "VATUSA division pull failed: 502 Bad Gateway");
        assert_eq!(body.users_reset, 0);
        assert_eq!(everything(&w.pool).await, before);
    }

    /// A reset needs a reason, checked before the pull runs.
    #[sqlx::test]
    async fn a_reset_needs_a_reason(pool: PgPool) {
        let w = world(pool).await;
        let before = everything(&w.pool).await;
        let result = reset_to_vatusa(&w.state, &admin_user(&w), "  ", None, async {
            panic!("the pull must not run without a reason")
        })
        .await;
        assert!(matches!(result, Err(ResetError::Api(ApiError::BadRequest))));
        assert_eq!(everything(&w.pool).await, before);
    }

    /// AC7: a member whose reset fails mid-way is rolled back whole, the run stops, the response says
    /// how many were reset, and every member after it is untouched — their hand-made rows, `system`
    /// rows and detach all intact.
    #[sqlx::test]
    async fn a_failure_part_way_leaves_no_member_half_reset(pool: PgPool) {
        let w = world(pool).await;
        let broken = user(&w.pool, 1_795_003, "Broken").await;
        detach(&w.pool, &broken).await;
        group(&w.pool, &broken, "ACE", None, "manual").await;
        grant(&w.pool, &broken, "access.users.read", None).await;
        let later = user(&w.pool, 1_795_004, "Later").await;
        detach(&w.pool, &later).await;
        group(&w.pool, &later, "ACE", None, "manual").await;
        group(&w.pool, &later, "NTMO", None, "system").await;
        grant(&w.pool, &later, "access.users.read", None).await;
        let later_before = rows(&w.pool, &later).await;
        let broken_before = rows(&w.pool, &broken).await;

        // Deleting the broken member's hand-made permission fails, after their groups are deleted and
        // their detach cleared in the same transaction.
        sqlx::query(
            "create function public.refuse_reset() returns trigger language plpgsql as \
             $$ begin raise exception 'refused'; end $$",
        )
        .execute(&w.pool)
        .await
        .unwrap();
        sqlx::query(&format!(
            "create trigger refuse_broken before delete on access.user_permissions \
             for each row when (old.user_id = '{broken}') execute function public.refuse_reset()"
        ))
        .execute(&w.pool)
        .await
        .unwrap();

        let mut nudges = w.state.events.subscribe();
        let Err(ResetError::Failed(status, body)) = reset(&w).await else {
            panic!("the run must report the failure");
        };
        let nudge = nudges
            .try_recv()
            .expect("members already reset are announced even when the run stops");
        assert_eq!(nudge.topic, crate::realtime::topic::ACCESS_GRANTED);
        assert_eq!(status, http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.error, "reset_incomplete");
        assert_eq!(
            body.users_reset, 2,
            "the admin and the member, before the broken one"
        );

        assert_eq!(rows(&w.pool, &w.member).await, member_after());
        assert_eq!(rows(&w.pool, &broken).await, broken_before);
        assert!(is_detached(&w.pool, &broken).await);
        assert_eq!(audits(&w.pool, &broken).await, []);
        assert_eq!(rows(&w.pool, &later).await, later_before);
        assert!(is_detached(&w.pool, &later).await);
    }

    /// AC1, on the real route: only the server admin may run either endpoint. A national
    /// `access.users.update` holder — who may Resync one member — is refused both, and nothing moves.
    #[sqlx::test]
    async fn only_the_server_admin_may_reset(pool: PgPool) {
        let w = world(pool).await;
        let editor = user(&w.pool, 1_795_010, "Editor").await;
        grant(&w.pool, &editor, "access.users.update", None).await;
        grant(&w.pool, &editor, "access.users.read", None).await;
        let editor_cookie = session_cookie(&w.pool, &editor).await;
        let plain_cookie = session_cookie(&w.pool, &w.steady).await;
        let before = everything(&w.pool).await;

        for cookie in [&editor_cookie, &plain_cookie] {
            let get = send(
                &w.state,
                http::Method::GET,
                "/api/v1/admin/access/vatusa-reset",
                cookie,
                None,
            )
            .await;
            let post = send(
                &w.state,
                http::Method::POST,
                "/api/v1/admin/access/vatusa-reset",
                cookie,
                Some(json!({"reason": "reset"})),
            )
            .await;
            assert_eq!(
                (get, post),
                (http::StatusCode::FORBIDDEN, http::StatusCode::FORBIDDEN)
            );
        }
        let anonymous = send(
            &w.state,
            http::Method::GET,
            "/api/v1/admin/access/vatusa-reset",
            "",
            None,
        )
        .await;
        assert_eq!(anonymous, http::StatusCode::UNAUTHORIZED);
        assert_eq!(everything(&w.pool).await, before);

        // Positive control: the same route answers the server admin.
        let (status, _) = send_json(
            &w.state,
            http::Method::GET,
            "/api/v1/admin/access/vatusa-reset",
            &w.admin_cookie,
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
    }

    /// The server admin's POST reaches the apply handler, and the reason they sent reaches its check:
    /// a blank reason is refused with 400 before anything is pulled or reset. Bound to the dry-run
    /// handler, the same request would answer 200. That the handler passes the payload's reason
    /// itself, not some other non-blank text, is pinned by `the_route_pulls_with_the_configured_key`.
    #[sqlx::test]
    async fn the_admins_post_refuses_a_blank_reason(pool: PgPool) {
        let w = world(pool).await;
        let before = everything(&w.pool).await;
        let status = send(
            &w.state,
            http::Method::POST,
            "/api/v1/admin/access/vatusa-reset",
            &w.admin_cookie,
            Some(json!({"reason": "  "})),
        )
        .await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
        assert_eq!(everything(&w.pool).await, before);
    }

    /// Only members a reset can change are examined, and every kind of drift is found: an attached
    /// member with only a hand-made group, only a hand-made permission, only a stale `vatusa` grant, or
    /// only a VATUSA-justified grant they lack, and a member with no CID. A member already in line is
    /// left alone.
    #[sqlx::test]
    async fn every_kind_of_drift_is_found(pool: PgPool) {
        let w = world(pool).await;
        let only_group = user(&w.pool, 1_795_020, "Only Group").await;
        group(&w.pool, &only_group, "ACE", None, "manual").await;
        let only_permission = user(&w.pool, 1_795_021, "Only Permission").await;
        grant(&w.pool, &only_permission, "access.users.read", None).await;
        let only_stale = user(&w.pool, 1_795_022, "Only Stale").await;
        group(&w.pool, &only_stale, "AEC", Some("ZDC"), "vatusa").await;
        let only_missing = user(&w.pool, 1_795_023, "Only Missing").await;
        vatusa_role(&w.pool, 1_795_023, "ZDC", "EVENT_COORDINATOR").await;
        let no_cid: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('No CID', 'No CID') \
             returning id",
        )
        .fetch_one(&w.pool)
        .await
        .unwrap();
        grant(&w.pool, &no_cid, "access.users.read", None).await;

        let body = reset(&w).await.ok().unwrap();

        for (who, left) in [
            (&only_group, vec![]),
            (&only_permission, vec![]),
            (&only_stale, vec![]),
            (
                &only_missing,
                row("group", "EC", Some("ZDC"), "vatusa", true),
            ),
            (&no_cid, vec![]),
        ] {
            assert_eq!(rows(&w.pool, who).await, left, "{who}");
            assert_eq!(audits(&w.pool, who).await.len(), 1, "{who}");
        }
        let names: Vec<&str> = body.users.iter().map(|u| u.display_name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Admin",
                "Member",
                "Only Group",
                "Only Permission",
                "Only Stale",
                "Only Missing",
                "No CID"
            ]
        );
        assert_eq!(body.users_checked, 8, "every member is counted");
        assert_eq!(audits(&w.pool, &w.steady).await, []);
    }

    /// AC5 and AC7 surface to the admin through this body: the cause and how many were reset.
    #[tokio::test]
    async fn a_failure_reaches_the_admin_with_its_cause_and_count() {
        let response = ResetError::Failed(
            http::StatusCode::BAD_GATEWAY,
            AccessResetFailure {
                error: "vatusa_pull_failed".into(),
                message: "VATUSA division pull failed: 502".into(),
                users_reset: 3,
            },
        )
        .into_response();
        assert_eq!(response.status(), http::StatusCode::BAD_GATEWAY);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body,
            json!({"error": "vatusa_pull_failed", "message": "VATUSA division pull failed: 502", "users_reset": 3})
        );
    }

    /// The route's own pull reads the configured key, and the route starts the run with the reason the
    /// admin sent. A source scan, because a test cannot set `VATUSA_API_KEY` without racing every other
    /// test, and a configured key would reach VATUSA.
    #[test]
    fn the_route_pulls_with_the_configured_key() {
        let source = include_str!("access_reset.rs");
        let handler = &source[source.find("pub async fn apply_vatusa_reset").unwrap()..];
        let handler = &handler[..handler.find("\n}\n").unwrap()];
        assert!(
            handler.contains("let Some(api_key) = crate::config::vatusa_api_key() else"),
            "apply_vatusa_reset must refuse when VATUSA is not configured"
        );
        assert!(
            handler.contains("division_pull(pool.clone(), state.events.clone(), api_key)"),
            "apply_vatusa_reset must pull with the configured VATUSA key"
        );
        assert!(handler.contains("start_reset("));
        assert!(
            handler.contains("&payload.reason,"),
            "apply_vatusa_reset must reset with the reason the admin sent"
        );
    }

    /// A member already in line comes back from `reset_member` as unchanged, so a reset never audits
    /// them even if one reached them: a positive control first, then the in-line member.
    #[sqlx::test]
    async fn reset_member_reports_no_change_for_a_member_in_line(pool: PgPool) {
        let w = world(pool).await;
        let mut tx = w.pool.begin().await.unwrap();
        assert!(
            vatusa_repo::reset_member(&mut tx, &w.member)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            vatusa_repo::reset_member(&mut tx, &w.steady)
                .await
                .unwrap()
                .is_none()
        );
        tx.rollback().await.unwrap();
    }

    /// The display names of the members a reset would examine, in the order it would.
    async fn candidates(pool: &PgPool) -> Vec<String> {
        let mut names = Vec::new();
        for id in vatusa_repo::reset_candidates(pool).await.unwrap() {
            names.push(
                sqlx::query_scalar("select display_name from identity.users where id = $1")
                    .bind(&id)
                    .fetch_one(pool)
                    .await
                    .unwrap(),
            );
        }
        names
    }

    /// A reset examines exactly the members `reset_member` would change, from every source VATUSA
    /// justifies a grant through: a role mapping, the roster's home ARTCC, a visiting ARTCC, and a
    /// division (`ZHQ`) role, which is national. A member in line through each source is left out, and
    /// so is one whose only hand-made grant is the protected baseline, or whose only direct permission
    /// is a `system` one. A held grant that differs from
    /// the justified one only in its scope, or only in its group, is still drift.
    #[sqlx::test]
    async fn a_reset_examines_exactly_the_members_it_would_change(pool: PgPool) {
        let w = world(pool).await;
        let home_missing = user(&w.pool, 1_795_030, "Home Missing").await;
        let home_ok = user(&w.pool, 1_795_031, "Home Ok").await;
        // Each holds one `vatusa` row that matches what VATUSA justifies (CONTROLLER at ZDC) in all
        // but one column: the scope for one, the group for the other.
        let wrong_scope = user(&w.pool, 1_795_037, "Wrong Scope").await;
        let wrong_group = user(&w.pool, 1_795_038, "Wrong Group").await;
        sqlx::query("update identity.users set home_facility = 'ZDC' where id = any($1)")
            .bind([&home_missing, &home_ok, &wrong_scope, &wrong_group])
            .execute(&w.pool)
            .await
            .unwrap();
        group(&w.pool, &home_ok, "CONTROLLER", Some("ZDC"), "vatusa").await;
        group(&w.pool, &wrong_scope, "CONTROLLER", Some("ZJX"), "vatusa").await;
        group(&w.pool, &wrong_group, "EC", Some("ZDC"), "vatusa").await;
        user(&w.pool, 1_795_032, "Visit Missing").await;
        let visit_ok = user(&w.pool, 1_795_033, "Visit Ok").await;
        for cid in [1_795_032_i64, 1_795_033] {
            sqlx::query("insert into identity.vatusa_visits (cid, facility) values ($1, 'ZJX')")
                .bind(cid)
                .execute(&w.pool)
                .await
                .unwrap();
        }
        group(&w.pool, &visit_ok, "CONTROLLER", Some("ZJX"), "vatusa").await;
        user(&w.pool, 1_795_034, "Division Missing").await;
        let division_ok = user(&w.pool, 1_795_035, "Division Ok").await;
        for cid in [1_795_034_i64, 1_795_035] {
            vatusa_role(&w.pool, cid, "ZHQ", "DIVISION_TECH_TEAM").await;
        }
        group(&w.pool, &division_ok, "VATUSA_STAFF", None, "vatusa").await;
        let baseline_only = user(&w.pool, 1_795_036, "Baseline Only").await;
        group(&w.pool, &baseline_only, "USER", None, "manual").await;
        let system_permission = user(&w.pool, 1_795_039, "System Permission").await;
        permission(
            &w.pool,
            &system_permission,
            "access.users.read",
            None,
            "system",
            true,
        )
        .await;

        assert_eq!(
            candidates(&w.pool).await,
            [
                "Admin",
                "Member",
                "Home Missing",
                "Visit Missing",
                "Division Missing",
                "Wrong Scope",
                "Wrong Group"
            ]
        );

        // Parity: a member is examined exactly when resetting them would change something.
        let users: Vec<(String, String)> =
            sqlx::query_as("select id, display_name from identity.users order by cid")
                .fetch_all(&w.pool)
                .await
                .unwrap();
        let examined = candidates(&w.pool).await;
        for (id, name) in &users {
            let mut tx = w.pool.begin().await.unwrap();
            let changes = vatusa_repo::reset_member(&mut tx, id)
                .await
                .unwrap()
                .is_some();
            tx.rollback().await.unwrap();
            assert_eq!(examined.contains(name), changes, "{name}");
        }

        // The admin is examined only for the hand-made EC: their manual USER and SERVER_ADMIN don't
        // count.
        sqlx::query("delete from access.user_roles where user_id = $1 and role_name = 'EC'")
            .bind(&w.admin)
            .execute(&w.pool)
            .await
            .unwrap();
        assert!(!candidates(&w.pool).await.contains(&"Admin".to_string()));
    }

    // --- The run off the request (#806) ---

    /// A pull that says when the run reached it, then waits until the test lets it through.
    fn gated_pull() -> (
        impl Future<Output = Result<String, PullError>> + Send + 'static,
        tokio::sync::oneshot::Receiver<()>,
        std::sync::Arc<tokio::sync::Notify>,
    ) {
        let (reached, reached_rx) = tokio::sync::oneshot::channel();
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let gate = release.clone();
        let pull = async move {
            let _ = reached.send(());
            gate.notified().await;
            Ok("pulled".to_string())
        };
        (pull, reached_rx, release)
    }

    async fn start(
        w: &World,
        pull: impl Future<Output = Result<String, PullError>> + Send + 'static,
    ) -> Uuid {
        // Starting returns at once, whatever the run is waiting on.
        let started = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            start_reset(&w.state, &admin_user(w), "back to VATUSA", None, pull),
        )
        .await
        .expect("start_reset waited for the run");
        let run_id = started.ok().expect("the reset starts");
        Uuid::parse_str(&run_id).unwrap()
    }

    async fn latest_run(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "select id from access.vatusa_reset_runs order by started_at desc limit 1",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The run once it has finished.
    async fn finished(pool: &PgPool, id: Uuid) -> AccessResetRun {
        for _ in 0..600 {
            let run = reset_runs::fetch_run(pool, id).await.unwrap().unwrap();
            if run.status != "running" {
                return run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("the run did not finish within 30 s");
    }

    async fn status_of(pool: &PgPool, id: Uuid) -> String {
        reset_runs::fetch_run(pool, id)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    fn reset_job(w: &World) -> Option<crate::job_registry::JobStatus> {
        w.state
            .jobs
            .snapshot()
            .into_iter()
            .find(|j| j.name == RESET_JOB)
    }

    /// Long enough for a run that is not held back to have reached its pull.
    const SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

    /// #806 AC: the client goes away mid-run and the run still completes, resets every member and
    /// publishes `ACCESS_GRANTED`. The request's future is aborted while the run waits in its pull, as
    /// hyper drops a handler's future when its connection closes; run inline in the request, the reset
    /// would stop there.
    #[sqlx::test]
    async fn a_dropped_client_does_not_stop_the_run(pool: PgPool) {
        let w = world(pool).await;
        let mut nudges = w.state.events.subscribe();
        let (pull, reached, release) = gated_pull();
        let (state, admin) = (w.state.clone(), admin_user(&w));
        let request = tokio::spawn(async move {
            let _ = start_reset(&state, &admin, "back to VATUSA", None, pull).await;
            std::future::pending::<()>().await;
        });
        reached.await.expect("the run reaches its pull");
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        release.notify_one();

        let run = finished(&w.pool, latest_run(&w.pool).await).await;
        assert_eq!(
            run.status,
            "succeeded",
            "{:?}",
            run.failure.map(|f| f.message)
        );
        let result = run.result.expect("a finished run stores its result");
        assert_eq!(result.users_reset, 2);
        assert_eq!(result.users_checked, 3);
        assert_eq!(result.pull_summary.as_deref(), Some("pulled"));
        assert_eq!(rows(&w.pool, &w.member).await, member_after());
        let nudge = nudges.try_recv().expect("the run tells signed-in browsers");
        assert_eq!(nudge.topic, crate::realtime::topic::ACCESS_GRANTED);
    }

    /// #806 AC: the run is in the job registry: running while it runs, then finished with its count.
    #[sqlx::test]
    async fn a_run_is_listed_in_the_job_registry(pool: PgPool) {
        let w = world(pool).await;
        assert!(reset_job(&w).is_none(), "registered before any run");
        let (pull, reached, release) = gated_pull();
        let id = start(&w, pull).await;
        reached.await.unwrap();
        let running = reset_job(&w).expect("a started run is listed");
        assert!(running.running);
        assert!(!running.triggerable && running.interval_secs.is_none());
        release.notify_one();
        finished(&w.pool, id).await;

        let done = reset_job(&w).unwrap();
        assert!(!done.running);
        assert_eq!(done.runs, 1);
        assert_eq!(done.last_ok, Some(true));
        assert_eq!(done.last_detail.as_deref(), Some("2 of 3 members reset"));
    }

    /// A pull that fails is the run's failure, stored for the dialog and recorded in the registry, and
    /// nothing is reset.
    #[sqlx::test]
    async fn a_failed_pull_fails_the_run(pool: PgPool) {
        let w = world(pool).await;
        let before = everything(&w.pool).await;
        let id = start(&w, async {
            Err(PullError::Failed(
                "VATUSA division pull failed: 502 Bad Gateway".to_string(),
            ))
        })
        .await;
        let run = finished(&w.pool, id).await;
        assert_eq!(run.status, "failed");
        assert!(run.result.is_none());
        let failure = run.failure.unwrap();
        assert_eq!(failure.error, "vatusa_pull_failed");
        assert_eq!(
            failure.message,
            "VATUSA division pull failed: 502 Bad Gateway"
        );
        assert_eq!(failure.users_reset, 0);
        let job = reset_job(&w).unwrap();
        assert_eq!(job.last_ok, Some(false));
        assert_eq!(
            job.last_detail.as_deref(),
            Some("VATUSA division pull failed: 502 Bad Gateway")
        );
        assert_eq!(everything(&w.pool).await, before);
    }

    /// #806 AC: a reset does not pull while the division pull holds the division lock; it starts once
    /// the pull lets go.
    #[sqlx::test]
    async fn a_reset_waits_for_the_division_pull(pool: PgPool) {
        let w = world(pool).await;
        let division_pull = reset_runs::lock_division(&w.pool).await.unwrap();
        let (pull, mut reached, release) = gated_pull();
        let id = start(&w, pull).await;
        tokio::time::sleep(SETTLE).await;
        assert!(
            reached.try_recv().is_err(),
            "the reset pulled while the division pull ran"
        );
        assert_eq!(status_of(&w.pool, id).await, "running");

        drop(division_pull);
        reached
            .await
            .expect("the reset pulls once the division pull is done");
        release.notify_one();
        assert_eq!(finished(&w.pool, id).await.status, "succeeded");
    }

    /// #806 AC: the division pull, through the lock its job takes, waits for a running reset and runs
    /// once the reset has finished.
    #[sqlx::test]
    async fn the_division_pull_waits_for_a_reset(pool: PgPool) {
        let w = world(pool).await;
        let (pull, reached, release) = gated_pull();
        let id = start(&w, pull).await;
        reached.await.unwrap();

        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let division_pull = tokio::spawn({
            let (pool, ran) = (w.pool.clone(), ran.clone());
            async move {
                crate::feed::vatusa::with_division_lock(&pool, async {
                    ran.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
                .await
            }
        });
        tokio::time::sleep(SETTLE).await;
        assert!(
            !ran.load(std::sync::atomic::Ordering::SeqCst),
            "the division pull ran during a reset"
        );

        release.notify_one();
        assert_eq!(finished(&w.pool, id).await.status, "succeeded");
        division_pull.await.unwrap().unwrap();
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// The division pull job's body runs under the division lock. A source scan, because the job's pull
    /// reaches VATUSA; `the_division_pull_waits_for_a_reset` proves the lock itself.
    #[test]
    fn the_division_pull_job_takes_the_division_lock() {
        let source = include_str!("../feed/vatusa.rs");
        let job = &source[source.find("pub fn spawn_division_pull").unwrap()..];
        let job = &job[..job.find("\n}\n").unwrap()];
        let locked = job
            .find("with_division_lock(&pool, async {")
            .expect("spawn_division_pull must pull under the division lock");
        let pulled = job.find("pull_division(&pool").unwrap();
        assert!(locked < pulled, "the pull must run inside the locked block");
    }

    /// One reset at a time: a second start while one runs is refused with the running run's id, and a
    /// start after it finished goes ahead.
    #[sqlx::test]
    async fn a_second_reset_is_refused_while_one_runs(pool: PgPool) {
        let w = world(pool).await;
        let (pull, reached, release) = gated_pull();
        let first = start(&w, pull).await;
        reached.await.unwrap();

        let second = start_reset(&w.state, &admin_user(&w), "again", None, pulled()).await;
        let Err(ResetError::Running(running)) = second else {
            panic!("a second reset must be refused while one runs");
        };
        assert_eq!(running, first.to_string());
        let response = ResetError::Running(running).into_response();
        assert_eq!(response.status(), http::StatusCode::CONFLICT);
        let count: i64 = sqlx::query_scalar("select count(*) from access.vatusa_reset_runs")
            .fetch_one(&w.pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "the refused start recorded a run");

        release.notify_one();
        assert_eq!(finished(&w.pool, first).await.status, "succeeded");
        let third = start(&w, pulled()).await;
        assert_eq!(finished(&w.pool, third).await.status, "succeeded");
    }

    /// The run's route answers the server admin with the run, and refuses everyone else.
    #[sqlx::test]
    async fn the_run_is_read_through_its_route(pool: PgPool) {
        let w = world(pool).await;
        let id = start(&w, pulled()).await;
        finished(&w.pool, id).await;
        let uri = format!("/api/v1/admin/access/vatusa-reset/runs/{id}");

        let (status, run) = send_json(&w.state, http::Method::GET, &uri, &w.admin_cookie).await;
        assert_eq!(status, http::StatusCode::OK, "{run}");
        assert_eq!(run["id"], id.to_string());
        assert_eq!(run["status"], "succeeded");
        assert_eq!(run["result"]["users_reset"], 2);
        assert_eq!(run["failure"], serde_json::Value::Null);

        let editor = user(&w.pool, 1_795_010, "Editor").await;
        grant(&w.pool, &editor, "access.users.update", None).await;
        let editor_cookie = session_cookie(&w.pool, &editor).await;
        assert_eq!(
            send(&w.state, http::Method::GET, &uri, &editor_cookie, None).await,
            http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&w.state, http::Method::GET, &uri, "", None).await,
            http::StatusCode::UNAUTHORIZED
        );
        for missing in [Uuid::new_v4().to_string(), "not-a-run".to_string()] {
            let (status, _) = send_json(
                &w.state,
                http::Method::GET,
                &format!("/api/v1/admin/access/vatusa-reset/runs/{missing}"),
                &w.admin_cookie,
            )
            .await;
            assert_eq!(status, http::StatusCode::NOT_FOUND, "{missing}");
        }
    }

    /// With `VATUSA_API_KEY` unset, the admin's POST is refused with 503 and starts no run.
    #[sqlx::test]
    async fn an_unconfigured_vatusa_starts_nothing(pool: PgPool) {
        let w = world(pool).await;
        let before = everything(&w.pool).await;
        let status = send(
            &w.state,
            http::Method::POST,
            "/api/v1/admin/access/vatusa-reset",
            &w.admin_cookie,
            Some(json!({"reason": "back to VATUSA"})),
        )
        .await;
        assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE);
        let runs: i64 = sqlx::query_scalar("select count(*) from access.vatusa_reset_runs")
            .fetch_one(&w.pool)
            .await
            .unwrap();
        assert_eq!(runs, 0);
        assert!(reset_job(&w).is_none());
        assert_eq!(everything(&w.pool).await, before);
    }

    /// A run left `running` by a backend that died holds no lock: it reads as interrupted, with the
    /// members its audit entries show it reset, and the next reset closes it. A live run, whose task
    /// holds the lock, still reads as running.
    #[sqlx::test]
    async fn a_run_left_by_a_dead_backend_reads_as_interrupted(pool: PgPool) {
        let w = world(pool).await;
        let dead: Uuid = sqlx::query_scalar(
            "insert into access.vatusa_reset_runs (started_by, reason, started_at) \
             values ($1, 'back to VATUSA', now() - interval '1 minute') returning id",
        )
        .bind(&w.admin)
        .fetch_one(&w.pool)
        .await
        .unwrap();
        // The dead run got as far as the admin and the member.
        reset(&w).await.ok().unwrap();

        let run = reset_runs::fetch_run(&w.pool, dead).await.unwrap().unwrap();
        assert_eq!(run.status, "failed");
        assert!(run.finished_at.is_none());
        let failure = run.failure.unwrap();
        assert_eq!(failure.error, "reset_interrupted");
        assert_eq!(failure.users_reset, 2);

        let (pull, reached, release) = gated_pull();
        let live = start(&w, pull).await;
        reached.await.unwrap();
        assert_eq!(status_of(&w.pool, live).await, "running");
        let stored: (String, bool) = sqlx::query_as(
            "select status, finished_at is not null from access.vatusa_reset_runs where id = $1",
        )
        .bind(dead)
        .fetch_one(&w.pool)
        .await
        .unwrap();
        assert_eq!(
            stored,
            ("failed".to_string(), true),
            "the next reset closes it"
        );
        assert_eq!(
            reset_runs::fetch_run(&w.pool, dead)
                .await
                .unwrap()
                .unwrap()
                .failure
                .unwrap()
                .users_reset,
            2
        );

        release.notify_one();
        assert_eq!(finished(&w.pool, live).await.status, "succeeded");
    }
}
