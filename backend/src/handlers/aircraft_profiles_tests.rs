//! VATUSA/OIS#818: engine-family aircraft profiles through the real router — session resolution,
//! `RequirePermission`, `normalize`, the repo write and the force-reload of
//! `state.aircraft_profiles` all on the path.
//!
//! B772 is a jet in `feed::flow::ENGINE_REF` with no 0059 seed row, so with no type row added a
//! family row is what moves it off the wake/default tiers. Descent rates are literals (2_345), never read
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
    assert_eq!(label(&state, "B772", ""), "default");
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

/// Aircraft profiles are national reference data: the handler has no ARTCC check and
/// `RequirePermission` is scope-blind, so a facility-scoped holder writes a family row exactly as
/// it writes a type or wake row today. #818 does not change who may write.
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
    assert_eq!(label(&state, "B772", ""), "default", "precondition");

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
    assert_eq!(table.resolve("B772", "").desc_fpm, 2_345.0);
    assert_eq!(table.resolve_label("B772", ""), "family:jet");
    assert_eq!(
        table.resolve_label("B772", "H"),
        "family:jet",
        "family beats the 0059 wake H row"
    );
    assert_eq!(
        table.resolve_label("B738", ""),
        "type:B738",
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
        state.aircraft_profiles.load().resolve("B772", "").desc_fpm,
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
    assert_eq!(label(&state, "B772", ""), "default");

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
    // Every key the route stores is one resolution reaches: PA34 (piston), AT72 (turboprop) and
    // B772 (jet), none with a type row of its own.
    let table = state.aircraft_profiles.load();
    for (ty, label) in [
        ("PA34", "family:piston"),
        ("AT72", "family:turboprop"),
        ("B772", "family:jet"),
    ] {
        assert_eq!(table.resolve_label(ty, ""), label, "{ty}");
    }
}

#[sqlx::test]
async fn delete_family_removes_only_that_row_and_the_cache_stops_resolving_to_it(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let holder = user_with(&pool, &[UPDATE], None).await;
    for (key, fpm) in [("jet", 2_345.0), ("turboprop", 2_777.0)] {
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
    assert_eq!(label(&state, "B772", ""), "family:jet", "precondition");

    let status = send(
        &state,
        Method::DELETE,
        &format!("{BASE}/family/jet"),
        &holder,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        family_rows(&pool).await,
        [("turboprop".to_string(), 2_777.0)]
    );
    let same_key: i64 = sqlx::query_scalar(
        "select count(*) from flow.aircraft_profile where kind = 'type' and key = 'jet'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(same_key, 1, "the type row keyed 'jet' stays");
    assert_eq!(label(&state, "B772", ""), "default");
    assert_eq!(label(&state, "AT72", ""), "family:turboprop");
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
