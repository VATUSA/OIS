//! VATUSA/OIS#818: engine-family aircraft profiles through the real router — session resolution,
//! `RequirePermission`, `normalize`, the repo write and the force-reload of
//! `state.aircraft_profiles` all on the path.
//!
//! E175 is a jet and DA20 a piston in `feed::flow::ENGINE_REF` with no type row and no ERAM record
//! (#985), so a family row is what moves them off the wake/default tiers. Descent rates are literals (2_345), never read
//! back from a constant.
//!
//! Coverage matrix (VATUSA/OIS#818)
//!
//! Requirement                                | Test
//! -------------------------------------------|-------------------------------------------------------------------------
//! AC2 add: unauthenticated refused           | put_family_refuses_an_unauthenticated_caller
//! AC2 add: no permission refused             | put_family_refuses_a_caller_without_the_update_permission
//! AC2 add: facility-scoped holder            | put_family_by_a_zdc_scoped_holder_is_accepted_as_national_data
//! AC2 add/edit + cache reload + AC1 order    | put_family_stores_the_row_lowercase_and_the_cache_resolves_a_jet_to_it
//! AC2 add: unknown family refused            | put_family_refuses_an_unknown_engine_family
//! AC2 delete + cache reload + neighbors      | delete_family_removes_only_that_row_and_the_cache_stops_resolving_to_it
//! AC2 delete: unknown / missing family       | delete_family_refuses_an_unknown_family_and_404s_a_missing_one
//! AC2 list                                   | get_lists_a_family_row_between_the_wake_and_type_rows

use std::collections::HashMap;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state},
    state::AppState,
};

const READ: &str = "flow.aircraft_profiles.read";
const UPDATE: &str = "flow.aircraft_profiles.update";
const BASE: &str = "/api/v1/flow/aircraft-profiles";

fn body(desc_fpm: f64) -> Value {
    json!({
        "climb_ias_lo": 250, "climb_ias_hi": 290, "climb_fpm_lo": 2000, "climb_fpm_hi": 1500,
        "service_ceiling_ft": 41000, "desc_ias_hi": 290, "desc_ias_lo": 250, "desc_fpm": desc_fpm,
    })
}

/// A signed-in user holding `perms`, each at `artcc` (None = national).
async fn user_with(pool: &PgPool, perms: &[&str], artcc: Option<&str>) -> String {
    let user = seed_user(pool).await;
    for p in perms {
        grant(pool, &user, p, artcc).await;
    }
    session_cookie(pool, &user).await
}

/// Every stored family row as `(key, desc_fpm)`, by key.
async fn family_rows(pool: &PgPool) -> Vec<(String, f64)> {
    sqlx::query_as(
        "select key, desc_fpm from flow.aircraft_profile where kind = 'family' order by key",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

fn label(state: &AppState, ty: &str, wake: &str) -> String {
    state.aircraft_profiles.load().resolve_label(ty, wake)
}

#[sqlx::test]
async fn put_family_refuses_an_unauthenticated_caller(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());

    let status = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/jet"),
        "",
        Some(body(2_345.0)),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(family_rows(&pool).await.is_empty());
    let holder = user_with(&pool, &[UPDATE], None).await;
    let control = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/jet"),
        &holder,
        Some(body(2_345.0)),
    )
    .await;
    assert_eq!(
        control,
        StatusCode::OK,
        "positive control: a holder adds it"
    );
}

#[sqlx::test]
async fn put_family_refuses_a_caller_without_the_update_permission(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let reader = user_with(&pool, &[READ], None).await;

    let status = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/jet"),
        &reader,
        Some(body(2_345.0)),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(family_rows(&pool).await.is_empty());
    assert_eq!(label(&state, "E175", ""), "default");
    let holder = user_with(&pool, &[UPDATE], None).await;
    let control = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/jet"),
        &holder,
        Some(body(2_345.0)),
    )
    .await;
    assert_eq!(
        control,
        StatusCode::OK,
        "positive control: a holder adds it"
    );
}

/// Characterizes current behavior, not the intended contract: the handler has no ARTCC check and
/// `RequirePermission` is scope-blind, so a facility-scoped holder writes a family row exactly as
/// it writes a type or wake row. #818 does not change who may write; #1038 decides whether a
/// scoped holder should, and flips this test if not.
#[sqlx::test]
async fn put_family_by_a_zdc_scoped_holder_is_accepted_as_national_data(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let zdc = user_with(&pool, &[UPDATE], Some("ZDC")).await;

    let status = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/jet"),
        &zdc,
        Some(body(2_345.0)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(family_rows(&pool).await, [("jet".to_string(), 2_345.0)]);
}

#[sqlx::test]
async fn put_family_stores_the_row_lowercase_and_the_cache_resolves_a_jet_to_it(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let holder = user_with(&pool, &[UPDATE], None).await;
    assert_eq!(label(&state, "E175", ""), "default", "precondition");

    let add = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/JET"),
        &holder,
        Some(body(2_345.0)),
    )
    .await;

    assert_eq!(add, StatusCode::OK);
    assert_eq!(family_rows(&pool).await, [("jet".to_string(), 2_345.0)]);
    let table = state.aircraft_profiles.load();
    assert_eq!(table.row("E175", "").desc_fpm, 2_345.0);
    assert_eq!(table.resolve_label("E175", ""), "family:jet");
    assert_eq!(
        table.resolve_label("E175", "H"),
        "family:jet",
        "family beats the 0059 wake H row"
    );
    assert_eq!(
        table.resolve_label("PA28", ""),
        "type:PA28",
        "its own row beats family"
    );

    let edit = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/jet"),
        &holder,
        Some(body(2_456.0)),
    )
    .await;

    assert_eq!(edit, StatusCode::OK);
    assert_eq!(family_rows(&pool).await, [("jet".to_string(), 2_456.0)]);
    assert_eq!(
        state.aircraft_profiles.load().row("E175", "").desc_fpm,
        2_456.0
    );
}

#[sqlx::test]
async fn put_family_refuses_an_unknown_engine_family(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let holder = user_with(&pool, &[UPDATE], None).await;

    for key in ["turbofan", "B738", "jets"] {
        let status = send(
            &state,
            Method::PUT,
            &format!("{BASE}/family/{key}"),
            &holder,
            Some(body(2_345.0)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{key}");
    }
    assert!(family_rows(&pool).await.is_empty());
    assert_eq!(label(&state, "E175", ""), "default");

    for key in ["Piston", "turboprop", "jet"] {
        let status = send(
            &state,
            Method::PUT,
            &format!("{BASE}/family/{key}"),
            &holder,
            Some(body(2_345.0)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "positive control: {key}");
    }
    let keys: Vec<String> = family_rows(&pool).await.into_iter().map(|r| r.0).collect();
    assert_eq!(keys, ["jet", "piston", "turboprop"]);
    // Every key the route stores is one resolution reaches. Every turboprop in `ENGINE_REF` has an
    // ERAM record, which comes before the families.
    let table = state.aircraft_profiles.load();
    for (ty, label) in [
        ("DA20", "family:piston"),
        ("AT72", "eram:AT72"),
        ("E175", "family:jet"),
    ] {
        assert_eq!(table.resolve_label(ty, ""), label, "{ty}");
    }
}

#[sqlx::test]
async fn delete_family_removes_only_that_row_and_the_cache_stops_resolving_to_it(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let holder = user_with(&pool, &[UPDATE], None).await;
    for (key, fpm) in [("jet", 2_345.0), ("piston", 2_777.0)] {
        let s = send(
            &state,
            Method::PUT,
            &format!("{BASE}/family/{key}"),
            &holder,
            Some(body(fpm)),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }
    // The same-key neighbor in another kind; no route stores a lowercase type key.
    sqlx::query(
        "insert into flow.aircraft_profile (kind, key, desc_fpm) values ('type', 'jet', 1234)",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(label(&state, "E175", ""), "family:jet", "precondition");

    let status = send(
        &state,
        Method::DELETE,
        &format!("{BASE}/family/jet"),
        &holder,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(family_rows(&pool).await, [("piston".to_string(), 2_777.0)]);
    let same_key: i64 = sqlx::query_scalar(
        "select count(*) from flow.aircraft_profile where kind = 'type' and key = 'jet'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(same_key, 1, "the type row keyed 'jet' stays");
    assert_eq!(label(&state, "E175", ""), "default");
    assert_eq!(label(&state, "DA20", ""), "family:piston");
}

#[sqlx::test]
async fn delete_family_refuses_an_unknown_family_and_404s_a_missing_one(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let holder = user_with(&pool, &[UPDATE], None).await;
    let reader = user_with(&pool, &[READ], None).await;

    let bad = send(
        &state,
        Method::DELETE,
        &format!("{BASE}/family/turbofan"),
        &holder,
        None,
    )
    .await;
    let missing = send(
        &state,
        Method::DELETE,
        &format!("{BASE}/family/piston"),
        &holder,
        None,
    )
    .await;
    let unauthorized = send(
        &state,
        Method::DELETE,
        &format!("{BASE}/family/piston"),
        &reader,
        None,
    )
    .await;

    assert_eq!(bad, StatusCode::BAD_REQUEST);
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(unauthorized, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn get_lists_a_family_row_between_the_wake_and_type_rows(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let holder = user_with(&pool, &[UPDATE], None).await;
    let s = send(
        &state,
        Method::PUT,
        &format!("{BASE}/family/jet"),
        &holder,
        Some(body(2_345.0)),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let reader = user_with(&pool, &[READ], None).await;
    let nobody = user_with(&pool, &[], None).await;

    let (status, list) = send_json(&state, Method::GET, BASE, &reader).await;

    assert_eq!(status, StatusCode::OK, "{list}");
    let rows = list.as_array().expect("an array");
    let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    let family: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "family").collect();
    assert_eq!(family.len(), 1);
    assert_eq!(family[0]["key"], "jet");
    assert_eq!(family[0]["desc_fpm"], 2_345.0);
    let at = kinds.iter().position(|k| *k == "family").unwrap();
    let last_wake = kinds.iter().rposition(|k| *k == "wake").unwrap();
    let first_type = kinds.iter().position(|k| *k == "type").unwrap();
    assert!(last_wake < at && at < first_type, "{kinds:?}");

    let (refused, _) = send_json(&state, Method::GET, BASE, &nobody).await;
    assert_eq!(refused, StatusCode::UNAUTHORIZED);
}

// ---- #985: the write handler's force-reload keeps the ERAM table -------------------------------
//
// Coverage matrix (VATUSA/OIS#985)
//
// Requirement                         | Test
// ------------------------------------|---------------------------------------------------------------
// AC 5 a type row overrides ERAM      | put_type_takes_an_eram_type_off_eram_and_delete_returns_it
// AC 5 refused write changes nothing  | a_refused_type_write_leaves_the_eram_resolution_as_it_was

// AC 5. Through the real router by a national holder of flow.aircraft_profiles.update: precondition (after a
// refresh) "eram:B738"; PUT type/B738 (desc_fpm 2,345) -> 200 and the cache resolves "type:B738" with 2,345;
// DELETE -> 204 and it is "eram:B738" again, not "wake" or "default". Control: A320 "eram:A320" throughout.
// Red when: the handler's reload builds a table without ERAM.
#[sqlx::test]
async fn put_type_takes_an_eram_type_off_eram_and_delete_returns_it(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    state.aircraft_profiles.store(std::sync::Arc::new(
        crate::repos::aircraft_profiles::load_all(&pool)
            .await
            .unwrap(),
    ));
    let holder = user_with(&pool, &[UPDATE], None).await;
    assert_eq!(label(&state, "B738", "M"), "eram:B738", "precondition");

    let url = format!("{BASE}/type/B738");
    let put = send(&state, Method::PUT, &url, &holder, Some(body(2_345.0))).await;
    assert_eq!(put, StatusCode::OK);
    assert_eq!(label(&state, "B738", "M"), "type:B738");
    assert_eq!(
        state.aircraft_profiles.load().row("B738", "M").desc_fpm,
        2_345.0
    );
    assert_eq!(
        label(&state, "A320", "M"),
        "eram:A320",
        "its neighbor stays"
    );

    let delete = send(&state, Method::DELETE, &url, &holder, None).await;
    assert_eq!(delete, StatusCode::NO_CONTENT);
    assert_eq!(label(&state, "B738", "M"), "eram:B738");
    assert_eq!(label(&state, "A320", "M"), "eram:A320");
}

// AC 5. Unauthenticated, and a holder of read only, PUT type/B738: 401 each, no row stored, "eram:B738" still.
// Control: an update holder's PUT is 200. Red when: RequirePermission is dropped from the route.
#[sqlx::test]
async fn a_refused_type_write_leaves_the_eram_resolution_as_it_was(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    state.aircraft_profiles.store(std::sync::Arc::new(
        crate::repos::aircraft_profiles::load_all(&pool)
            .await
            .unwrap(),
    ));
    let reader = user_with(&pool, &[READ], None).await;
    let url = format!("{BASE}/type/B738");
    for cookie in ["", reader.as_str()] {
        let status = send(&state, Method::PUT, &url, cookie, Some(body(2_345.0))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(label(&state, "B738", "M"), "eram:B738");
    }
    let stored: i64 = sqlx::query_scalar(
        "select count(*) from flow.aircraft_profile where kind = 'type' and key = 'B738'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, 0);

    let holder = user_with(&pool, &[UPDATE], None).await;
    let control = send(&state, Method::PUT, &url, &holder, Some(body(2_345.0))).await;
    assert_eq!(control, StatusCode::OK, "positive control");
    assert_eq!(label(&state, "B738", "M"), "type:B738");
}
