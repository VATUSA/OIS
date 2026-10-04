//! VATUSA/OIS#584: a service account can be granted specific permissions at specific ARTCCs, its
//! effective access honours that `artcc_id`, an admin can never grant one more than they hold, and
//! its credentials expire.
//!
//! The escalation tests go through the real router, so `RequirePermission`, the handler's cap and the
//! repo write are all on the path — a cap that stops being called fails here, not just in a unit.

use std::collections::HashMap;

use axum::http::Method;
use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::PgPool;

use crate::repos::access::{
    self as access_repo, PermissionScope, fetch_service_account_permission_names,
    service_account_permission_scope,
};
use crate::repos::service_accounts as sa_repo;
use crate::scope_test_support::{grant, seed_user, send, session_cookie, test_state};

const FCA: &str = "flow.fca.update";
const CFR: &str = "tmu.cfr.assign";
const ROLE: &str = "T584_ROLE";

async fn account(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ('t584', 'T584') returning id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn direct_grant(pool: &PgPool, id: &str, permission: &str, artcc: Option<&str>) {
    sqlx::query(
        "insert into access.service_account_permissions (service_account_id, permission_name, artcc_id) \
         values ($1, $2, $3)",
    )
    .bind(id)
    .bind(permission)
    .bind(artcc)
    .execute(pool)
    .await
    .unwrap();
}

/// A role granting `permissions`, assignable to a service account.
async fn role(pool: &PgPool, permissions: &[&str]) {
    sqlx::query("insert into access.roles (name, is_system) values ($1, false)")
        .bind(ROLE)
        .execute(pool)
        .await
        .unwrap();
    for permission in permissions {
        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) values ($1, $2)",
        )
        .bind(ROLE)
        .bind(permission)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// A signed-in admin who may manage service accounts and holds FCA at ZDC only.
async fn zdc_admin(pool: &PgPool) -> String {
    let admin = seed_user(pool).await;
    grant(pool, &admin, "service_accounts.update", None).await;
    grant(pool, &admin, FCA, Some("ZDC")).await;
    session_cookie(pool, &admin).await
}

async fn grants_of(pool: &PgPool, id: &str) -> Vec<(String, Option<String>)> {
    sa_repo::fetch_permissions(pool, id)
        .await
        .unwrap()
        .into_iter()
        .map(|p| (p.permission, p.artcc_id))
        .collect()
}

// --- AC2: effective access honours artcc_id ---

#[sqlx::test]
async fn a_direct_grant_reaches_only_its_artcc(pool: PgPool) {
    let id = account(&pool).await;
    direct_grant(&pool, &id, FCA, Some("ZDC")).await;

    let names = fetch_service_account_permission_names(&pool, &id)
        .await
        .unwrap();
    assert_eq!(
        names,
        vec![FCA.to_string()],
        "the gate sees the direct grant"
    );

    let scope = service_account_permission_scope(&pool, &id, FCA)
        .await
        .unwrap();
    assert!(scope.allows(Some("ZDC")));
    assert!(!scope.allows(Some("ZNY")), "a ZDC grant must not reach ZNY");
    assert!(!scope.allows(None), "nor a resource with no owning ARTCC");
}

#[sqlx::test]
async fn roles_and_direct_grants_are_one_set(pool: PgPool) {
    let id = account(&pool).await;
    role(&pool, &[CFR]).await;
    sa_repo::set_roles(&pool, &id, &[ROLE.to_string()])
        .await
        .unwrap();
    direct_grant(&pool, &id, FCA, Some("ZDC")).await;
    direct_grant(&pool, &id, CFR, Some("ZNY")).await;

    let names = fetch_service_account_permission_names(&pool, &id)
        .await
        .unwrap();
    assert_eq!(names, vec![FCA.to_string(), CFR.to_string()]);
    // The role is national, so it wins over the narrower direct grant of the same permission.
    assert!(matches!(
        service_account_permission_scope(&pool, &id, CFR)
            .await
            .unwrap(),
        PermissionScope::National { .. }
    ));
}

// --- AC3: no escalation past the acting admin (adversarial) ---

#[sqlx::test]
async fn an_admin_can_grant_exactly_what_they_hold(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let cookie = zdc_admin(&pool).await;
    let id = account(&pool).await;
    let uri = format!("/api/v1/admin/service-accounts/{id}/permissions");

    let status = send(
        &state,
        Method::PUT,
        &uri,
        &cookie,
        Some(json!({"permissions": [{"permission": FCA, "artcc_id": "ZDC"}]})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        grants_of(&pool, &id).await,
        vec![(FCA.to_string(), Some("ZDC".to_string()))]
    );
}

#[sqlx::test]
async fn an_admin_cannot_grant_beyond_their_own_scope(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let cookie = zdc_admin(&pool).await;
    let id = account(&pool).await;
    let uri = format!("/api/v1/admin/service-accounts/{id}/permissions");

    for (permissions, why) in [
        (
            json!([{"permission": FCA, "artcc_id": "ZNY"}]),
            "another facility",
        ),
        (
            json!([{"permission": FCA, "artcc_id": null}]),
            "national, from a ZDC-only admin",
        ),
        (
            json!([{"permission": CFR, "artcc_id": "ZDC"}]),
            "a permission they don't hold",
        ),
        // Smuggled in beside a grant that is allowed: the whole replace is refused.
        (
            json!([{"permission": FCA, "artcc_id": "ZDC"}, {"permission": FCA, "artcc_id": "ZNY"}]),
            "a mixed request",
        ),
    ] {
        let status = send(
            &state,
            Method::PUT,
            &uri,
            &cookie,
            Some(json!({"permissions": permissions})),
        )
        .await;
        assert_eq!(status, 403, "{why} must be refused");
    }
    assert!(
        grants_of(&pool, &id).await.is_empty(),
        "nothing was written"
    );
}

#[sqlx::test]
async fn a_role_cannot_carry_what_the_admin_lacks(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let cookie = zdc_admin(&pool).await;
    let id = account(&pool).await;
    let uri = format!("/api/v1/admin/service-accounts/{id}/roles");

    // FCA is in the role nationally, but this admin holds it only at ZDC.
    role(&pool, &[FCA]).await;
    let status = send(
        &state,
        Method::PUT,
        &uri,
        &cookie,
        Some(json!({"role_names": [ROLE]})),
    )
    .await;
    assert_eq!(status, 403, "a role is the bypass if it isn't capped");
    assert!(
        access_repo::fetch_service_account_role_names(&pool, &id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test]
async fn a_role_within_the_admins_authority_is_granted(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let admin = seed_user(&pool).await;
    grant(&pool, &admin, "service_accounts.update", None).await;
    grant(&pool, &admin, FCA, None).await;
    let cookie = session_cookie(&pool, &admin).await;
    let id = account(&pool).await;

    role(&pool, &[FCA]).await;
    let uri = format!("/api/v1/admin/service-accounts/{id}/roles");
    let status = send(
        &state,
        Method::PUT,
        &uri,
        &cookie,
        Some(json!({"role_names": [ROLE]})),
    )
    .await;
    assert_eq!(status, 200);
}

/// Even a server admin, who holds everything, cannot give a machine the power to mint credentials.
#[sqlx::test]
async fn no_one_can_grant_a_machine_credential_management(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let admin = seed_user(&pool).await;
    sqlx::query("insert into access.user_roles (user_id, role_name) values ($1, 'SERVER_ADMIN')")
        .bind(&admin)
        .execute(&pool)
        .await
        .unwrap();
    let cookie = session_cookie(&pool, &admin).await;
    let id = account(&pool).await;
    let uri = format!("/api/v1/admin/service-accounts/{id}/permissions");

    for permission in [
        "api_keys.key.create",
        "service_accounts.create",
        "service_accounts.update",
    ] {
        let status = send(
            &state,
            Method::PUT,
            &uri,
            &cookie,
            Some(json!({"permissions": [{"permission": permission}]})),
        )
        .await;
        assert_eq!(
            status, 400,
            "{permission} must never reach a service account"
        );
    }
    assert!(grants_of(&pool, &id).await.is_empty());
}

/// Whoever rotates receives the token, so rotating is acquiring the account's authority. A ZDC admin
/// may rotate an account that reaches only ZDC, never one that reaches ZNY.
#[sqlx::test]
async fn an_admin_cannot_rotate_an_account_that_outranks_them(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let cookie = zdc_admin(&pool).await;

    let zny = account(&pool).await;
    direct_grant(&pool, &zny, FCA, Some("ZNY")).await;
    let status = send(
        &state,
        Method::POST,
        &format!("/api/v1/admin/service-accounts/{zny}/rotate"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(
        status, 403,
        "rotating would hand a ZDC admin a ZNY credential"
    );
    let live: i64 = sqlx::query_scalar(
        "select count(*) from access.service_account_credentials where service_account_id = $1",
    )
    .bind(&zny)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 0, "no credential was issued");

    let zdc: String = sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ('zdc', 'ZDC') returning id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    direct_grant(&pool, &zdc, FCA, Some("ZDC")).await;
    let status = send(
        &state,
        Method::POST,
        &format!("/api/v1/admin/service-accounts/{zdc}/rotate"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, 200);
}

/// A role widened after it was assigned (or assigned before roles were capped) must still never let a
/// machine hold credential management: the denylist applies when a request is authorised, too.
#[sqlx::test]
async fn a_forbidden_permission_reached_through_a_role_is_never_held(pool: PgPool) {
    let id = account(&pool).await;
    role(
        &pool,
        &[FCA, "service_accounts.update", "api_keys.key.create"],
    )
    .await;
    // Straight to the repo: this is the state an old or widened assignment leaves behind.
    sa_repo::set_roles(&pool, &id, &[ROLE.to_string()])
        .await
        .unwrap();

    let names = fetch_service_account_permission_names(&pool, &id)
        .await
        .unwrap();
    assert_eq!(
        names,
        vec![FCA.to_string()],
        "only the allowed permission survives"
    );
    assert!(
        service_account_permission_scope(&pool, &id, "service_accounts.update")
            .await
            .unwrap()
            .is_empty()
    );
}

// --- AC4: expiry and staleness ---

#[sqlx::test]
async fn a_new_credential_expires_in_90_days_by_default_and_never_past_365(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let admin = seed_user(&pool).await;
    grant(&pool, &admin, "service_accounts.create", None).await;
    let cookie = session_cookie(&pool, &admin).await;
    let uri = "/api/v1/admin/service-accounts";

    for days in [0, 366] {
        let status = send(
            &state,
            Method::POST,
            uri,
            &cookie,
            Some(json!({"name": "too long", "expires_in_days": days})),
        )
        .await;
        assert_eq!(status, 400, "{days} days must be refused");
    }

    let status = send(
        &state,
        Method::POST,
        uri,
        &cookie,
        Some(json!({"name": "default"})),
    )
    .await;
    assert_eq!(status, 200);
    let accounts = sa_repo::list_service_accounts(&pool).await.unwrap();
    assert_eq!(accounts.len(), 1, "the refused requests created nothing");
    let expires = accounts[0]
        .expires_at
        .expect("a new credential has an expiry");
    let expected = Utc::now() + Duration::days(90);
    assert!(
        (expires - expected).num_minutes().abs() < 5,
        "about 90 days out, got {expires}"
    );
}

#[sqlx::test]
async fn an_expired_credential_no_longer_authenticates(pool: PgPool) {
    let id = account(&pool).await;
    let token = "ois_sa_t584_expired";
    sqlx::query(
        "insert into access.service_account_credentials (service_account_id, secret_hash, expires_at) \
         values ($1, $2, now() - interval '1 minute')",
    )
    .bind(&id)
    .bind(access_repo::sha256_hex(token))
    .execute(&pool)
    .await
    .unwrap();

    let found = access_repo::find_current_service_account_by_bearer_token(&pool, token)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn a_credential_unused_for_30_days_is_reported_stale(pool: PgPool) {
    let fresh = account(&pool).await;
    sa_repo::rotate_credential(&pool, &fresh, "fresh", Utc::now() + Duration::days(90))
        .await
        .unwrap();
    let idle: String = sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ('idle', 'Idle') returning id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into access.service_account_credentials \
         (service_account_id, secret_hash, created_at, last_used_at, expires_at) \
         values ($1, 'idle', now() - interval '60 days', now() - interval '31 days', now() + interval '30 days')",
    )
    .bind(&idle)
    .execute(&pool)
    .await
    .unwrap();

    let get = |id: String| {
        let pool = pool.clone();
        async move {
            sa_repo::get_service_account(&pool, &id)
                .await
                .unwrap()
                .unwrap()
        }
    };
    assert!(
        !get(fresh).await.stale,
        "a credential issued today is not stale"
    );
    assert!(get(idle).await.stale, "31 days unused is stale");
}

// --- Removal is gated like granting (the #546 ruling, applied here) ---

async fn assign_role(pool: &PgPool, id: &str) {
    sqlx::query(
        "insert into access.service_account_roles (service_account_id, role_name) values ($1, $2)",
    )
    .bind(id)
    .bind(ROLE)
    .execute(pool)
    .await
    .unwrap();
}

async fn roles_of(pool: &PgPool, id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "select role_name from access.service_account_roles where service_account_id = $1",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Both replaces used to delete everything and check only what they inserted, so a facility admin
/// could switch off a national integration by sending an empty list.
#[sqlx::test]
async fn a_scoped_admin_cannot_strip_what_they_could_not_grant(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let cookie = zdc_admin(&pool).await;
    let id = account(&pool).await;
    direct_grant(&pool, &id, FCA, Some("ZNY")).await;
    direct_grant(&pool, &id, CFR, None).await;
    role(&pool, &[CFR]).await;
    assign_role(&pool, &id).await;

    for (permissions, why) in [
        (json!([]), "everything"),
        (
            json!([{"permission": CFR, "artcc_id": null}]),
            "another facility's grant",
        ),
        (
            json!([{"permission": FCA, "artcc_id": "ZNY"}]),
            "a permission they don't hold",
        ),
    ] {
        let status = send(
            &state,
            Method::PUT,
            &format!("/api/v1/admin/service-accounts/{id}/permissions"),
            &cookie,
            Some(json!({"permissions": permissions})),
        )
        .await;
        assert_eq!(status, 403, "removing {why} must be refused");
    }
    let status = send(
        &state,
        Method::PUT,
        &format!("/api/v1/admin/service-accounts/{id}/roles"),
        &cookie,
        Some(json!({"role_names": []})),
    )
    .await;
    assert_eq!(
        status, 403,
        "removing a role carrying what they lack must be refused"
    );

    assert_eq!(
        grants_of(&pool, &id).await,
        vec![
            (FCA.to_string(), Some("ZNY".to_string())),
            (CFR.to_string(), None)
        ],
        "nothing was removed"
    );
    assert_eq!(roles_of(&pool, &id).await, vec![ROLE.to_string()]);
}

/// The other side: grants beyond the admin pass through untouched, and what is within their authority
/// they can still add and remove — the check is on what changes, not on the whole account.
#[sqlx::test]
async fn a_scoped_admin_edits_around_grants_beyond_them(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let cookie = zdc_admin(&pool).await;
    let id = account(&pool).await;
    direct_grant(&pool, &id, FCA, Some("ZNY")).await;
    role(&pool, &[CFR]).await;
    assign_role(&pool, &id).await;
    let uri = format!("/api/v1/admin/service-accounts/{id}/permissions");

    let status = send(
        &state,
        Method::PUT,
        &uri,
        &cookie,
        Some(json!({"permissions": [
            {"permission": FCA, "artcc_id": "ZNY"},
            {"permission": FCA, "artcc_id": "ZDC"}
        ]})),
    )
    .await;
    assert_eq!(
        status, 200,
        "adding within their authority, keeping ZNY as it was"
    );
    let mut held = grants_of(&pool, &id).await;
    held.sort();
    assert_eq!(
        held,
        vec![
            (FCA.to_string(), Some("ZDC".to_string())),
            (FCA.to_string(), Some("ZNY".to_string()))
        ]
    );

    let status = send(
        &state,
        Method::PUT,
        &uri,
        &cookie,
        Some(json!({"permissions": [{"permission": FCA, "artcc_id": "ZNY"}]})),
    )
    .await;
    assert_eq!(status, 200, "removing what they could grant");
    assert_eq!(
        grants_of(&pool, &id).await,
        vec![(FCA.to_string(), Some("ZNY".to_string()))]
    );

    let status = send(
        &state,
        Method::PUT,
        &format!("/api/v1/admin/service-accounts/{id}/roles"),
        &cookie,
        Some(json!({"role_names": [ROLE]})),
    )
    .await;
    assert_eq!(
        status, 200,
        "resubmitting a role they couldn't grant, unchanged, is a no-op"
    );
    assert_eq!(roles_of(&pool, &id).await, vec![ROLE.to_string()]);
}
