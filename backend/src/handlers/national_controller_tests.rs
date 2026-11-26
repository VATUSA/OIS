//! VATUSA/OIS#1121: a rostered VATUSA controller, home or visiting, uses `CONTROLLER`'s flow tools at
//! every ARTCC; every signed-in member reads FCAs and IDST; event planning and TMU publishing stay
//! with the roles that hold them. Each test drives the real router with principals built the way
//! production builds them: the VATUSA sync for a controller (`rostered_member`), the `USER` baseline
//! for a member.

use std::collections::HashMap;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::machine_actor_tests::{
    crossing_state_of, fca, seed_owned_release, seed_release, send_full, service_account,
};
use crate::repos::access::{GrantSource, set_user_role_scoped};
use crate::scope_test_support::{
    artcc, rostered_member, seed_user, session_cookie, signed_in_baseline,
};
use crate::state::AppState;

const IDST: &str = "/api/v1/flow/idst?airports=KJFK";

/// The credential entries for `cookie`: none for `""`, an anonymous caller.
fn entries(cookie: &str) -> Vec<&str> {
    if cookie.is_empty() {
        vec![]
    } else {
        vec![cookie]
    }
}

async fn get(state: &AppState, uri: &str, cookie: &str) -> (StatusCode, Value) {
    let (status, _, body) = send_full(state, Method::GET, uri, &entries(cookie), None).await;
    (status, body)
}

async fn status(
    state: &AppState,
    method: Method,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> StatusCode {
    send_full(state, method, uri, &entries(cookie), body)
        .await
        .0
}

/// The IDST row for `callsign`, released or not.
fn idst_row<'a>(idst: &'a Value, callsign: &str) -> &'a Value {
    ["unscheduled", "released"]
        .iter()
        .flat_map(|list| idst[*list].as_array().into_iter().flatten())
        .find(|row| row["callsign"] == callsign)
        .unwrap_or_else(|| panic!("no IDST row for {callsign}: {idst}"))
}

/// A member holding `role` by hand at `at` (or nationally), plus the signed-in baseline.
async fn with_role(pool: &PgPool, role: &str, at: Option<&str>) -> String {
    let user = seed_user(pool).await;
    signed_in_baseline(pool, &user).await;
    let mut tx = pool.begin().await.unwrap();
    set_user_role_scoped(&mut tx, &user, role, true, at, GrantSource::Manual)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    session_cookie(pool, &user).await
}

async fn member(pool: &PgPool) -> String {
    let user = seed_user(pool).await;
    signed_in_baseline(pool, &user).await;
    session_cookie(pool, &user).await
}

async fn released(pool: &PgPool, fca_id: &str, callsign: &str) -> bool {
    sqlx::query_scalar(
        "select exists(select 1 from flow.fca_release where fca_id = $1 and callsign = $2)",
    )
    .bind(fca_id)
    .bind(callsign)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// AC2 + AC4 + AC7: IDST's `can_release` is true exactly where the release route accepts the same
/// caller, least privileged first. The FCA is ZDC's.
#[sqlx::test]
async fn idst_can_release_matches_the_release_route_for_each_principal(pool: PgPool) {
    let state = crossing_state_of(pool.clone(), &["TEST1"]).await;
    let fca_id = fca(&pool).await;
    let release = format!("/api/v1/flow/fcas/{fca_id}/release/TEST1");

    // Unauthenticated: neither read nor write.
    assert_eq!(get(&state, IDST, "").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        status(&state, Method::POST, &release, "", Some(json!({}))).await,
        StatusCode::UNAUTHORIZED
    );

    // A signed-in member with only USER: reads IDST, may not release.
    let user = member(&pool).await;
    let (code, idst) = get(&state, IDST, &user).await;
    assert_eq!(code, StatusCode::OK, "{idst}");
    assert_eq!(idst_row(&idst, "TEST1")["can_release"], false);
    assert_eq!(
        status(&state, Method::POST, &release, &user, Some(json!({}))).await,
        StatusCode::UNAUTHORIZED
    );
    assert!(!released(&pool, &fca_id, "TEST1").await);

    // A CONTROLLER held by hand at ZNY only: the scope check still refuses ZDC's FCA.
    let zny = with_role(&pool, "CONTROLLER", Some("ZNY")).await;
    let (_, idst) = get(&state, IDST, &zny).await;
    assert_eq!(idst_row(&idst, "TEST1")["can_release"], false);
    assert_eq!(
        status(&state, Method::POST, &release, &zny, Some(json!({}))).await,
        StatusCode::FORBIDDEN
    );
    assert!(!released(&pool, &fca_id, "TEST1").await);

    // A rostered controller whose only ARTCC is ZTL: the sync's national CONTROLLER releases at ZDC.
    let ztl = rostered_member(&pool, "ZTL", &[]).await;
    let ztl = session_cookie(&pool, &ztl).await;
    let (_, idst) = get(&state, IDST, &ztl).await;
    assert_eq!(idst_row(&idst, "TEST1")["can_release"], true);
    let (code, _, body) = send_full(&state, Method::POST, &release, &[&ztl], Some(json!({}))).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert!(released(&pool, &fca_id, "TEST1").await);
    let (_, idst) = get(&state, IDST, &ztl).await;
    assert_eq!(idst_row(&idst, "TEST1")["released"], true);
    assert_eq!(idst_row(&idst, "TEST1")["can_release"], true);
    assert_eq!(
        status(&state, Method::DELETE, &release, &ztl, None).await,
        StatusCode::OK
    );
    assert!(!released(&pool, &fca_id, "TEST1").await);

    // An EC: at ZDC it may release ZDC's FCA; at ZNY only, it may not.
    let ec_zdc = with_role(&pool, "EC", Some("ZDC")).await;
    let (_, idst) = get(&state, IDST, &ec_zdc).await;
    assert_eq!(idst_row(&idst, "TEST1")["can_release"], true);
    let ec_zny = with_role(&pool, "EC", Some("ZNY")).await;
    let (_, idst) = get(&state, IDST, &ec_zny).await;
    assert_eq!(idst_row(&idst, "TEST1")["can_release"], false);
    assert_eq!(
        status(&state, Method::POST, &release, &ec_zny, Some(json!({}))).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(&state, Method::POST, &release, &ec_zdc, Some(json!({}))).await,
        StatusCode::OK
    );
}

/// AC7, the holder half: a machine may not change a person's release, so IDST says so for that row
/// and still offers the machine its own release and an unheld one.
#[sqlx::test]
async fn idst_can_release_follows_who_holds_the_release_for_a_machine(pool: PgPool) {
    let state = crossing_state_of(pool.clone(), &["TEST1", "TEST2", "TEST3"]).await;
    let fca_id = fca(&pool).await;
    let (sa, auth) = service_account(&pool, "flow.fca.update", None).await;
    sqlx::query(
        "insert into access.role_permissions (role_name, permission_name) \
         select role_name, 'flow.fca.read' from access.service_account_roles \
         where service_account_id = $1 on conflict do nothing",
    )
    .bind(&sa)
    .execute(&pool)
    .await
    .unwrap();
    let cta = Utc::now().timestamp_millis() + 600_000;
    seed_release(&pool, &fca_id, "TEST1", cta).await;
    seed_owned_release(&pool, &fca_id, "TEST2", cta + 120_000, &sa).await;

    let (code, _, idst) = send_full(&state, Method::GET, IDST, &[&auth], None).await;
    assert_eq!(code, StatusCode::OK, "{idst}");
    assert_eq!(idst_row(&idst, "TEST1")["can_release"], false, "a person's");
    assert_eq!(idst_row(&idst, "TEST2")["can_release"], true, "its own");
    assert_eq!(idst_row(&idst, "TEST3")["can_release"], true, "unheld");

    // The route agrees on the person's release.
    let (code, _, _) = send_full(
        &state,
        Method::DELETE,
        &format!("/api/v1/flow/fcas/{fca_id}/release/TEST1"),
        &[&auth, "If-Match: 1"],
        None,
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert!(released(&pool, &fca_id, "TEST1").await);
}

/// AC2: the rest of CONTROLLER's flow tools at another ARTCC. A controller whose only ARTCC is a ZNY
/// visit edits a ZDC route, sets KDCA's runways, excludes a flight from ZDC's FCA and assigns a CFR at
/// KDCA. A member with only USER is refused each (401: they hold none of the permissions).
#[sqlx::test]
async fn a_rostered_controller_works_another_artccs_flow_tools(pool: PgPool) {
    let state = crossing_state_of(pool.clone(), &["AAL1"]).await;
    *state.facilities.write().await = HashMap::from([
        ("ZDC".to_string(), artcc(&["KDCA"])),
        ("ZNY".to_string(), artcc(&["KJFK"])),
    ]);
    let fca_id = fca(&pool).await;
    let visitor = rostered_member(&pool, "", &["ZNY"]).await;
    let visitor = session_cookie(&pool, &visitor).await;
    let user = member(&pool).await;

    let route = json!({ "name": "ZDC route", "artcc": "ZDC", "route": "RBV WHITE SIE" });
    let exclusion = format!("/api/v1/flow/fcas/{fca_id}/exclusions/AAL1");
    let cfr = json!({
        "callsign": "AAL1",
        "airport": "KDCA",
        "ready_time": (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
    });
    let writes: [(Method, &str, Option<Value>); 4] = [
        (Method::POST, "/api/v1/flow/routes", Some(route)),
        (Method::PUT, "/api/v1/flow/runway/KDCA", Some(json!({}))),
        (Method::POST, &exclusion, Some(json!({ "reason": "test" }))),
        (Method::POST, "/api/v1/tmu/cfr", Some(cfr)),
    ];
    for (method, uri, body) in &writes {
        assert_eq!(
            status(&state, method.clone(), uri, &user, body.clone()).await,
            StatusCode::UNAUTHORIZED,
            "a member: {method} {uri}"
        );
    }
    for (method, uri, body) in &writes {
        let (code, _, resp) =
            send_full(&state, method.clone(), uri, &[&visitor], body.clone()).await;
        assert_eq!(
            code,
            StatusCode::OK,
            "the ZNY visitor: {method} {uri}: {resp}"
        );
    }
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "select (select count(*) from flow.route where artcc = 'ZDC'), \
                (select count(*) from flow.runway_config where icao = 'KDCA'), \
                (select count(*) from flow.manual_flight_exclusion where callsign = 'AAL1'), \
                (select count(*) from tmu.issued_cfrs where callsign = 'AAL1')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (1, 1, 1, 1), "each write landed once");
}

/// AC3, the TMU half: a rostered controller publishes nothing. Creating a TMI, an advisory, a ground
/// stop or a GDP, or setting a program, is refused for want of the permission (401). Positive
/// control: an NTMO gets past the same gate (to the body's validation, not 401).
#[sqlx::test]
async fn a_rostered_controller_cannot_publish_tmu_restrictions(pool: PgPool) {
    let state = crossing_state_of(pool.clone(), &[]).await;
    let controller = rostered_member(&pool, "ZDC", &["ZNY"]).await;
    let controller = session_cookie(&pool, &controller).await;
    let ntmo = with_role(&pool, "NTMO", None).await;
    let writes = [
        (Method::POST, "/api/v1/tmu/tmis"),
        (Method::POST, "/api/v1/tmu/advisories"),
        (Method::POST, "/api/v1/tmu/ground-stops"),
        (Method::POST, "/api/v1/tmu/gdp"),
        (Method::PUT, "/api/v1/tmu/programs/KDCA"),
    ];
    for (method, uri) in &writes {
        assert_eq!(
            status(&state, method.clone(), uri, &controller, Some(json!({}))).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri}"
        );
        assert_ne!(
            status(&state, method.clone(), uri, &ntmo, Some(json!({}))).await,
            StatusCode::UNAUTHORIZED,
            "the NTMO control: {method} {uri}"
        );
    }
    let written: i64 = sqlx::query_scalar(
        "select (select count(*) from tmu.tmis) + (select count(*) from tmu.ground_stops)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(written, 0);
}

/// AC2, the swap: a rostered controller whose only ARTCC is ZTL trades two releases on ZDC's FCA. A
/// member with only USER (401) and a CONTROLLER held by hand at ZNY (403) are refused first, and the
/// times stay where they were until the controller's swap.
#[sqlx::test]
async fn a_rostered_controller_swaps_releases_on_another_artccs_fca(pool: PgPool) {
    // Both off KJFK, on one runway, so the swap is otherwise allowed (#56).
    let state = crossing_state_of(pool.clone(), &["AAL1", "UAL2"]).await;
    for callsign in ["AAL1", "UAL2"] {
        crate::repos::departure_runway::assign(
            &pool,
            "KJFK",
            callsign,
            "31L",
            crate::repos::departure_runway::RunwaySource::Config,
            None,
        )
        .await
        .unwrap();
    }
    let fca_id = fca(&pool).await;
    seed_release(&pool, &fca_id, "AAL1", 1_000).await;
    seed_release(&pool, &fca_id, "UAL2", 2_000).await;
    let cta = || async {
        let v: i64 = sqlx::query_scalar(
            "select cta_ms from flow.fca_release where fca_id = $1 and callsign = 'AAL1'",
        )
        .bind(&fca_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        v
    };
    let swap = format!("/api/v1/flow/fcas/{fca_id}/swap");
    let body = || Some(json!({ "a": "AAL1", "b": "UAL2" }));

    let user = member(&pool).await;
    assert_eq!(
        status(&state, Method::POST, &swap, &user, body()).await,
        StatusCode::UNAUTHORIZED
    );
    let zny = with_role(&pool, "CONTROLLER", Some("ZNY")).await;
    assert_eq!(
        status(&state, Method::POST, &swap, &zny, body()).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(cta().await, 1_000, "refused swaps change nothing");

    let ztl = rostered_member(&pool, "ZTL", &[]).await;
    let ztl = session_cookie(&pool, &ztl).await;
    let (code, _, resp) = send_full(&state, Method::POST, &swap, &[&ztl], body()).await;
    assert_eq!(code, StatusCode::OK, "{resp}");
    assert_eq!(cta().await, 2_000, "swapped");
}
