//! Reset every member's access to exactly what VATUSA justifies (#795): a dry run, and the reset itself.
//! Server admin only. The gate is the `SERVER_ADMIN` group, not a catalog permission, so no group can
//! be given it.

use std::future::Future;

use axum::{
    Json,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::{
    auth::{
        acl::{fetch_user_access, is_server_admin},
        context::CurrentUser,
    },
    errors::ApiError,
    models::{
        AccessResetBody, AccessResetFailure, AccessResetGrant, AccessResetRequest, AccessResetUser,
    },
    repos::{
        audit as audit_repo,
        vatusa::{self as vatusa_repo, GrantRow, ResetMode, ResetRun},
    },
    state::AppState,
};

/// Why the reset endpoint refused or stopped: an ordinary API error, or a failure the admin is shown
/// with its cause and how many members were already reset.
pub enum ResetError {
    Api(ApiError),
    Failed(StatusCode, AccessResetFailure),
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
        (status = 200, body = AccessResetBody),
        (status = 400),
        (status = 401),
        (status = 403),
        (status = 500, description = "Stopped part-way; `users_reset` members were reset", body = AccessResetFailure),
        (status = 502, description = "The VATUSA division pull failed; nothing was reset", body = AccessResetFailure),
        (status = 503, description = "VATUSA is not configured; nothing was reset", body = AccessResetFailure)
    ),
    security(("session" = []))
)]
/// Reset every member's access to VATUSA (#795): pull the division fresh (refused with 503 when
/// VATUSA is not configured), then, one transaction per
/// member, put them back on role sync, delete every hand-made grant except the baseline `USER` and
/// `SERVER_ADMIN` groups, and reconcile their VATUSA grants. `system` grants are left alone. Each
/// changed member gets one `USER_ACCESS` audit entry with the reason. Because the pull is fresh, the
/// result can differ from the dry run if VATUSA changed since the last pull. Server admin only.
pub async fn apply_vatusa_reset(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Json(payload): Json<AccessResetRequest>,
) -> Result<Json<AccessResetBody>, ResetError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    require_server_admin(&state, user).await?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let pull = division_pull(pool, &state.events, crate::config::vatusa_api_key());
    reset_to_vatusa(
        &state,
        user,
        &payload.reason,
        audit_repo::client_ip(&headers),
        pull,
    )
    .await
    .map(Json)
}

/// Why the division pull that opens a reset did not run.
pub enum PullError {
    NotConfigured,
    Failed(String),
}

/// The division pull a reset opens with. With no VATUSA API key there is nothing to reset to, so it is
/// refused rather than run on whatever the last pull stored.
async fn division_pull(
    pool: &sqlx::PgPool,
    events: &crate::realtime::Events,
    api_key: Option<String>,
) -> Result<String, PullError> {
    let api_key = api_key.ok_or(PullError::NotConfigured)?;
    crate::feed::vatusa::pull_division(pool, &api_key, events)
        .await
        .map_err(PullError::Failed)
}

/// The reset behind [`apply_vatusa_reset`], with the division pull passed in so a test can stand in
/// for VATUSA. The pull runs first; if it fails, nothing is reset.
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
    let pull_summary = pull.await.map_err(|e| {
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
    })?;

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

        // With `VATUSA_API_KEY` unset the real pull refuses before reaching VATUSA.
        let unconfigured = reset_to_vatusa(
            &w.state,
            &admin_user(&w),
            "back to VATUSA",
            None,
            division_pull(&w.pool, &w.state.events, None),
        )
        .await;
        assert!(matches!(
            unconfigured,
            Err(ResetError::Failed(http::StatusCode::SERVICE_UNAVAILABLE, _))
        ));
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

    /// The server admin's POST reaches the apply handler and the reason they sent: a blank reason is
    /// refused with 400 before anything is pulled or reset. Bound to the dry-run handler, or given any
    /// reason but the payload's, the same request would answer 200 or reach the (unconfigured) pull.
    #[sqlx::test]
    async fn the_admins_post_applies_with_the_reason_sent(pool: PgPool) {
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

    /// The route's own pull reads the configured key. A source scan, because a test cannot set
    /// `VATUSA_API_KEY` without racing every other test, and a configured key would reach VATUSA.
    #[test]
    fn the_route_pulls_with_the_configured_key() {
        let source = include_str!("access_reset.rs");
        let handler = &source[source.find("pub async fn apply_vatusa_reset").unwrap()..];
        let handler = &handler[..handler.find("\n}\n").unwrap()];
        assert!(
            handler.contains("division_pull(pool, &state.events, crate::config::vatusa_api_key())"),
            "apply_vatusa_reset must pull with the configured VATUSA key"
        );
        assert!(handler.contains("reset_to_vatusa("));
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
}
