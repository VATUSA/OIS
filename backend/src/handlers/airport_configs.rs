//! Reusable per-airport runway configurations (default AAR/ADR + favored-wind rule). Reads are open
//! to planners (`events.plan.read`); writes are facility-scoped by the airport's owning ARTCC
//! (`events.config.update`), reusing the same scope infra as the per-event airport rates.

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{EventsConfigUpdate, EventsPlanRead},
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed,
    feed::facilities::FacilityMap,
    handlers::events::{normalize_icao, owning_artcc},
    models::{AirportConfigBody, AirportForecastBody, UpsertAirportConfigRequest},
    repos::access::PermissionScope,
    repos::airport_configs as config_repo,
    state::AppState,
};

const CONFIG_PERMISSION: &str = "events.config.update";

#[derive(Deserialize)]
pub struct ForecastQuery {
    /// Unix seconds of the time to forecast for; defaults to now.
    pub at: Option<i64>,
}

#[utoipa::path(
    get, path = "/api/v1/forecast/{icao}", tag = "events",
    params(("icao" = String, Path), ("at" = Option<i64>, Query)),
    responses((status = 200, body = AirportForecastBody), (status = 401))
)]
pub async fn forecast_wind(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(icao): Path<String>,
    Query(q): Query<ForecastQuery>,
) -> Result<Json<AirportForecastBody>, ApiError> {
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    let at =
        q.at.and_then(|s| Utc.timestamp_opt(s, 0).single())
            .unwrap_or_else(Utc::now);
    let airports = state.feed.read().await.airports.clone();
    match feed::forecast::wind_at(&airports, &icao, at).await {
        Some(h) => Ok(Json(AirportForecastBody {
            icao,
            time: h.time,
            wind_dir: h.dir,
            wind_kt: h.spd_kt,
            gust_kt: h.gust_kt,
            source: "forecast".to_string(),
        })),
        None => Ok(Json(AirportForecastBody {
            icao,
            time: at,
            wind_dir: None,
            wind_kt: 0,
            gust_kt: None,
            source: "none".to_string(),
        })),
    }
}

fn validate(req: &UpsertAirportConfigRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.name.len() > 64 {
        return Err(ApiError::BadRequest);
    }
    if !(0..=200).contains(&req.aar) || !(0..=200).contains(&req.adr) {
        return Err(ApiError::BadRequest);
    }
    if !(0..=360).contains(&req.wind_from_deg) || !(0..=360).contains(&req.wind_to_deg) {
        return Err(ApiError::BadRequest);
    }
    // #512 AC4: a rule must not name a runway this config does not have. Rejected rather than ignored
    // at read time, so the operator finds out when they save instead of wondering why their rule never
    // fires. Checked against the request's own `departure_runways`, since both can move together.
    for rules in [req.sid_rules.as_ref(), req.gate_rules.as_ref()]
        .into_iter()
        .flatten()
    {
        if !rules_fit(rules.values().map(String::as_str), &req.departure_runways) {
            return Err(ApiError::BadRequest);
        }
    }
    Ok(())
}

/// Whether every runway a rule map names is one the config actually has.
///
/// Comparison is on the trimmed, upper-cased name: a rule typed `31l` means the same runway as `31L`,
/// and rejecting it for case would be a trap rather than a safeguard.
fn rules_fit<'a>(mut named: impl Iterator<Item = &'a str>, available: &[String]) -> bool {
    let have: std::collections::HashSet<String> = available
        .iter()
        .map(|r| r.trim().to_ascii_uppercase())
        .collect();
    named.all(|r| have.contains(&r.trim().to_ascii_uppercase()))
}

/// Does the caller hold `events.config.update` nationally or for `icao`'s owning ARTCC?
/// Works for a signed-in user or an API key (whose scope is capped by its owner).
async fn can_edit(state: &AppState, principal: &Principal, icao: &str) -> Result<bool, ApiError> {
    let artcc = owning_artcc(state, icao).await;
    let scope = principal.permission_scope(state, CONFIG_PERMISSION).await?;
    Ok(scope.allows(artcc.as_deref()))
}

/// Fail-closed if the caller can't edit `icao`. Every write handler below calls this instead of
/// duplicating the `owning_artcc` + `permission_scope` + `scope.allows` check inline (#198: the
/// inline copies had silently drifted out of any test's reach).
async fn require_edit(state: &AppState, principal: &Principal, icao: &str) -> Result<(), ApiError> {
    if can_edit(state, principal, icao).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[derive(Deserialize)]
pub struct ConfigListQuery {
    /// Scope to one owning ARTCC; omit for every airport.
    pub artcc: Option<String>,
}

#[utoipa::path(
    get, path = "/api/v1/airport-configs", tag = "events",
    params(("artcc" = Option<String>, Query, description = "Scope to one owning ARTCC")),
    responses((status = 200, body = Vec<AirportConfigBody>), (status = 401))
)]
pub async fn list_all_airport_configs(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Query(q): Query<ConfigListQuery>,
) -> Result<Json<Vec<AirportConfigBody>>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = q
        .artcc
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_ascii_uppercase);

    let scope = principal
        .permission_scope(&state, CONFIG_PERMISSION)
        .await?;
    let rows = config_repo::list_all(pool).await?;
    let facilities = state.facilities.read().await;
    Ok(Json(annotate_and_filter(
        rows,
        &facilities,
        &scope,
        artcc.as_deref(),
    )))
}

/// Resolve each row's live owning ARTCC (not its stored snapshot — that can go stale after a
/// facility realignment) and use it for both `editable` and an optional `?artcc=` filter, matching
/// `can_edit`/`create`/`update` which always check live.
fn annotate_and_filter(
    rows: Vec<AirportConfigBody>,
    facilities: &FacilityMap,
    scope: &PermissionScope,
    artcc_filter: Option<&str>,
) -> Vec<AirportConfigBody> {
    rows.into_iter()
        .filter_map(|mut r| {
            let live = feed::facilities::artcc_for_airport(facilities, &r.icao);
            r.editable = scope.allows(live.as_deref());
            (artcc_filter.is_none() || live.as_deref() == artcc_filter).then_some(r)
        })
        .collect()
}

#[utoipa::path(
    get, path = "/api/v1/airport-configs/{icao}", tag = "events",
    params(("icao" = String, Path)),
    responses((status = 200, body = Vec<AirportConfigBody>), (status = 401))
)]
pub async fn list_airport_configs(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
) -> Result<Json<Vec<AirportConfigBody>>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;

    let editable = can_edit(&state, &principal, &icao).await?;
    let mut rows = config_repo::list_by_icao(pool, &icao).await?;
    for r in &mut rows {
        r.editable = editable;
    }
    Ok(Json(rows))
}

#[utoipa::path(
    post, path = "/api/v1/airport-configs/{icao}", tag = "events",
    params(("icao" = String, Path)), request_body = UpsertAirportConfigRequest,
    responses((status = 200, body = AirportConfigBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn create_airport_config(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsConfigUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
    Json(req): Json<UpsertAirportConfigRequest>,
) -> Result<Json<AirportConfigBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let artcc = owning_artcc(&state, &icao).await;
    let mut row = config_repo::create(
        pool,
        &icao,
        &req,
        artcc.as_deref().unwrap_or(""),
        principal.user_id(),
    )
    .await?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    put, path = "/api/v1/airport-configs/{icao}/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)), request_body = UpsertAirportConfigRequest,
    responses((status = 200, body = AirportConfigBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_airport_config(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsConfigUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
    Json(req): Json<UpsertAirportConfigRequest>,
) -> Result<Json<AirportConfigBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate(&req)?;
    require_edit(&state, &principal, &icao).await?;
    // The other half of AC4. `validate` only sees the rules the request carries; a save that omits them
    // while narrowing `departure_runways` would leave the STORED rules naming a runway that no longer
    // exists — orphaned silently, which is exactly what the AC forbids. Read them and check.
    if req.sid_rules.is_none() || req.gate_rules.is_none() {
        let stored = config_repo::get(pool, &id)
            .await?
            .ok_or(ApiError::NotFound)?;
        let keeping = [
            (req.sid_rules.is_none(), &stored.sid_rules),
            (req.gate_rules.is_none(), &stored.gate_rules),
        ];
        for (_, rules) in keeping.iter().filter(|(omitted, _)| *omitted) {
            if !rules_fit(rules.0.values().map(String::as_str), &req.departure_runways) {
                return Err(ApiError::BadRequest);
            }
        }
    }

    let mut row = config_repo::update(pool, &id, &icao, &req, principal.user_id())
        .await?
        .ok_or(ApiError::NotFound)?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/airport-configs/{icao}/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_airport_config(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsConfigUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    require_edit(&state, &principal, &icao).await?;

    if config_repo::delete(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use sqlx::PgPool;

    use super::*;
    use crate::feed::facilities::Facility;

    async fn seed_user(pool: &PgPool) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into identity.users (full_name, display_name) \
             values ('Test User', 'Test User') returning id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn upsert(name: &str) -> UpsertAirportConfigRequest {
        UpsertAirportConfigRequest {
            name: name.to_string(),
            aar: 30,
            adr: 30,
            landing_runways: vec![],
            departure_runways: vec![],
            // None, not empty: the default fixture must exercise the omit-to-keep path, which is what
            // every pre-#512 client sends.
            sid_rules: None,
            gate_rules: None,
            wind_from_deg: 0,
            wind_to_deg: 360,
            calm_default: false,
        }
    }

    /// #509 AC 3: `departure_runways` has to survive a create *and* an update. Both SQL statements
    /// list their columns positionally, so a column added to one and not the other, or bound out of
    /// order, writes the wrong value — and with two `text[]` columns side by side that silently swaps
    /// arrivals for departures rather than failing.
    #[sqlx::test]
    async fn departure_runways_round_trip_through_create_and_update(pool: PgPool) {
        let user = seed_user(&pool).await;
        let created = config_repo::create(
            &pool,
            "KJFK",
            &UpsertAirportConfigRequest {
                landing_runways: vec!["04R".into(), "22L".into()],
                departure_runways: vec!["04L".into(), "31L".into()],
                ..upsert("JFK south")
            },
            "ZNY",
            &user,
        )
        .await
        .unwrap();
        assert_eq!(created.departure_runways, vec!["04L", "31L"]);
        assert_eq!(
            created.landing_runways,
            vec!["04R", "22L"],
            "the two arrays must not be swapped"
        );

        let updated = config_repo::update(
            &pool,
            &created.id,
            "KJFK",
            &UpsertAirportConfigRequest {
                landing_runways: vec!["13L".into()],
                departure_runways: vec!["13R".into()],
                ..upsert("JFK south")
            },
            &user,
        )
        .await
        .unwrap()
        .expect("the config exists");
        assert_eq!(updated.departure_runways, vec!["13R"]);
        assert_eq!(updated.landing_runways, vec!["13L"]);
    }

    /// Realignment scenario: KORD's config was created while ZDC owned it (stored `artcc: "ZDC"`),
    /// but the live facility map now has it under ZAU. KDCA's stored value still matches live
    /// (ZDC, unaffected). KXXX has no resolvable owning ARTCC at all (absent from the facility map)
    /// — an empty-string stored `artcc` from a config created before any facility data existed.
    #[sqlx::test]
    async fn list_all_scopes_and_filters_by_the_live_artcc_not_the_stored_one(pool: PgPool) {
        let user = seed_user(&pool).await;
        config_repo::create(&pool, "KDCA", &upsert("DCA calm"), "ZDC", &user)
            .await
            .unwrap();
        config_repo::create(&pool, "KORD", &upsert("ORD calm"), "ZDC", &user)
            .await
            .unwrap(); // stale: stored ZDC, live (below) is ZAU
        config_repo::create(&pool, "KXXX", &upsert("XXX calm"), "", &user)
            .await
            .unwrap();

        let facilities = FacilityMap::from([
            (
                "ZDC".to_string(),
                Facility {
                    kind: "artcc".to_string(),
                    airports: vec!["KDCA".to_string()],
                },
            ),
            (
                "ZAU".to_string(),
                Facility {
                    kind: "artcc".to_string(),
                    airports: vec!["KORD".to_string()],
                },
            ),
        ]);

        // Ordering is untouched by annotate_and_filter — assert it directly on the raw fetch.
        let rows = config_repo::list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 3, "all three rows come back unfiltered");
        assert_eq!(
            rows.iter().map(|r| r.icao.as_str()).collect::<Vec<_>>(),
            ["KDCA", "KORD", "KXXX"],
            "icao, calm_default desc, name"
        );

        // A ZAU-scoped principal: editable only where the LIVE artcc is ZAU (KORD), despite its
        // stored value saying ZDC. The no-resolvable-ARTCC row (KXXX) is never editable under a
        // Facilities scope.
        let rows = config_repo::list_all(&pool).await.unwrap();
        let zau_scope = PermissionScope::Facilities(HashSet::from(["ZAU".to_string()]));
        let annotated = annotate_and_filter(rows, &facilities, &zau_scope, None);
        let editable: Vec<_> = annotated
            .iter()
            .filter(|r| r.editable)
            .map(|r| r.icao.as_str())
            .collect();
        assert_eq!(editable, ["KORD"]);

        // National scope sees everything as editable, live-artcc-filtered to ZDC: only KDCA, even
        // though KORD's *stored* value also says ZDC.
        let rows = config_repo::list_all(&pool).await.unwrap();
        let filtered =
            annotate_and_filter(rows, &facilities, &PermissionScope::National, Some("ZDC"));
        assert_eq!(
            filtered.iter().map(|r| r.icao.as_str()).collect::<Vec<_>>(),
            ["KDCA"]
        );
        assert!(filtered.iter().all(|r| r.editable));

        // A scope covering nobody's ARTCC: nothing is editable.
        let rows = config_repo::list_all(&pool).await.unwrap();
        let no_scope = PermissionScope::Facilities(HashSet::new());
        let annotated = annotate_and_filter(rows, &facilities, &no_scope, None);
        assert!(annotated.iter().all(|r| !r.editable));
    }

    // --- ARTCC-scope authorization boundary (#198) ---
    //
    // The test above (`annotate_and_filter`) exercises a *different*, list-only filtering
    // function against a plain `PermissionScope` value — it never calls `can_edit`/`require_edit`.
    // (create/update/delete_airport_config used to duplicate this check inline instead of calling
    // either helper, which meant these tests didn't actually cover the write path at all — fixed
    // alongside adding this coverage, so require_edit is now the single gate every write handler
    // calls.) These tests close that gap.

    use crate::scope_test_support::{self, artcc, grant, principal_for, test_state};

    #[sqlx::test]
    async fn national_scope_can_edit_any_artccs_airport(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", None).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(can_edit(&state, &principal, "KDCA").await.unwrap());
    }

    #[sqlx::test]
    async fn matching_artcc_scope_can_edit_its_own_airport(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(can_edit(&state, &principal, "KDCA").await.unwrap());
    }

    #[sqlx::test]
    async fn wrong_artcc_scope_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        // Granted for ZAU, but KDCA is owned by ZDC.
        grant(&pool, &user, "events.config.update", Some("ZAU")).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(!can_edit(&state, &principal, "KDCA").await.unwrap());
        assert!(matches!(
            require_edit(&state, &principal, "KDCA").await,
            Err(ApiError::Forbidden)
        ));
    }

    #[sqlx::test]
    async fn no_grant_at_all_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(!can_edit(&state, &principal, "KDCA").await.unwrap());
        assert!(matches!(
            require_edit(&state, &principal, "KDCA").await,
            Err(ApiError::Forbidden)
        ));
    }

    // --- Through the router (VATUSA/OIS#364) ---
    //
    // The tests above cover `can_edit`/`require_edit` directly, which stays green if a write handler
    // stops calling them. These send real requests with a real session, so each write's
    // `RequirePermission<EventsConfigUpdate>` and its `require_edit` call are on the tested path. A
    // missing permission is 401 and the wrong facility 403, so each test pins one gate.

    use http::{Method, StatusCode};
    use scope_test_support::{send, session_cookie};

    fn config_json() -> serde_json::Value {
        serde_json::json!({
            "name": "South flow",
            "aar": 36,
            "adr": 40,
            "landing_runways": ["19"],
            "wind_from_deg": 90,
            "wind_to_deg": 270
        })
    }

    /// A KDCA (ZDC) config to update and delete, and a session for `user`. Returns every write:
    /// create, update, delete.
    async fn routed(
        pool: PgPool,
        user: &str,
    ) -> (crate::state::AppState, String, [(Method, String); 3]) {
        let req: crate::models::UpsertAirportConfigRequest =
            serde_json::from_value(config_json()).unwrap();
        let existing = config_repo::create(&pool, "KDCA", &req, "ZDC", user)
            .await
            .unwrap();
        let cookie = session_cookie(&pool, user).await;
        let state = test_state(
            pool,
            std::collections::HashMap::from([
                ("ZDC".to_string(), artcc(&["KDCA"])),
                ("ZNY".to_string(), artcc(&["KJFK"])),
            ]),
        );
        let one = format!("/api/v1/airport-configs/KDCA/{}", existing.id);
        let writes = [
            (Method::POST, "/api/v1/airport-configs/KDCA".to_string()),
            (Method::PUT, one.clone()),
            (Method::DELETE, one),
        ];
        (state, cookie, writes)
    }

    async fn statuses(
        state: &crate::state::AppState,
        cookie: &str,
        writes: &[(Method, String); 3],
    ) -> Vec<StatusCode> {
        let mut out = Vec::new();
        for (method, uri) in writes {
            let body = (*method != Method::DELETE).then(config_json);
            out.push(send(state, method.clone(), uri, cookie, body).await);
        }
        out
    }

    /// Fails if any write drops `RequirePermission`: `require_edit` would then answer 403.
    #[sqlx::test]
    async fn through_the_router_a_caller_without_the_permission_gets_401(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let (state, cookie, writes) = routed(pool, &user).await;
        assert_eq!(
            statuses(&state, &cookie, &writes).await,
            [StatusCode::UNAUTHORIZED; 3]
        );
    }

    /// Fails if any write drops `require_edit`: the request would then go through.
    #[sqlx::test]
    async fn through_the_router_another_facilitys_grant_gets_403(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZNY")).await;
        let (state, cookie, writes) = routed(pool, &user).await;
        assert_eq!(
            statuses(&state, &cookie, &writes).await,
            [StatusCode::FORBIDDEN; 3]
        );
    }

    /// A map of rules, for the fixtures below.
    fn rules(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// AC3, and the one that protects the other 184 airports: a config created without rules has
    /// empty maps and behaves exactly as it did before #512.
    #[sqlx::test]
    async fn a_config_with_no_rules_is_unchanged(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let created = config_repo::create(&pool, "KJFK", &upsert("South"), "ZNY", &user)
            .await
            .unwrap();
        assert!(created.sid_rules.0.is_empty());
        assert!(created.gate_rules.0.is_empty());
    }

    /// AC2, the patch convention: a save that omits the rule maps leaves the stored ones alone. A
    /// client predating #512 sends exactly this shape on every unrelated edit, so without `coalesce`
    /// it would wipe an ARTCC's rules each time it changed an AAR.
    #[sqlx::test]
    async fn omitting_the_rules_leaves_them_unchanged(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "31L")]));
        let created = config_repo::create(&pool, "KJFK", &req, "ZNY", &user)
            .await
            .unwrap();

        // An unrelated edit, in the shape an older client sends: no rule fields at all.
        let mut later = upsert("South");
        later.departure_runways = vec!["31L".into()];
        later.aar = 44;
        assert!(later.sid_rules.is_none() && later.gate_rules.is_none());
        let updated = config_repo::update(&pool, &created.id, "KJFK", &later, &user)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.aar, 44, "the edit applied");
        assert_eq!(
            updated.sid_rules.0.get("CAMRN").map(String::as_str),
            Some("31L"),
            "and the rules it never mentioned survived it"
        );
    }

    /// AC1: rules round-trip, and the two maps stay distinct — a gate and a SID may share a name.
    #[sqlx::test]
    async fn rules_round_trip_per_config(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into(), "04L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "31L")]));
        req.gate_rules = Some(rules(&[("CAMRN", "04L")]));
        let row = config_repo::create(&pool, "KJFK", &req, "ZNY", &user)
            .await
            .unwrap();

        assert_eq!(
            row.sid_rules.0.get("CAMRN").map(String::as_str),
            Some("31L")
        );
        assert_eq!(
            row.gate_rules.0.get("CAMRN").map(String::as_str),
            Some("04L"),
            "a gate named like a SID is a different rule, which is why there are two maps"
        );
    }

    /// AC4: a rule naming a runway the config does not have is rejected, not stored and ignored.
    #[test]
    fn a_rule_naming_an_absent_runway_is_rejected() {
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "22R")]));
        assert!(matches!(validate(&req), Err(ApiError::BadRequest)));

        // The same runway, differently cased, is the same runway — rejecting it would be a trap.
        req.sid_rules = Some(rules(&[("CAMRN", "31l")]));
        assert!(validate(&req).is_ok());
    }

    /// AC4's other half, which `validate` alone cannot see: narrowing `departure_runways` while the
    /// request says nothing about the rules would orphan the STORED ones. The update handler reads them
    /// and refuses, so a config can never hold a rule pointing at a runway it no longer has.
    #[sqlx::test]
    async fn narrowing_the_runways_cannot_orphan_a_stored_rule(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZDC")).await;
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into(), "04L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "04L")]));
        let created = config_repo::create(&pool, "KDCA", &req, "ZDC", &user)
            .await
            .unwrap();

        // Drop 04L, saying nothing about the rules — the shape a pre-#512 client sends. The stored
        // CAMRN→04L rule would be left pointing at a runway the config no longer has.
        let narrowing = serde_json::json!({
            "name": "South",
            "aar": 30,
            "adr": 30,
            "landing_runways": [],
            "departure_runways": ["31L"],
            "wind_from_deg": 0,
            "wind_to_deg": 360
        });
        let cookie = session_cookie(&pool, &user).await;
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        let status = send(
            &state,
            Method::PUT,
            &format!("/api/v1/airport-configs/KDCA/{}", created.id),
            &cookie,
            Some(narrowing),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "narrowing the runways must not silently orphan the stored CAMRN rule"
        );
    }

    /// The positive control: ZDC's own editor gets through every write, so the refusals above are
    /// the gates answering and not a broken fixture.
    #[sqlx::test]
    async fn through_the_router_the_owning_facilitys_grant_goes_through(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZDC")).await;
        let (state, cookie, writes) = routed(pool, &user).await;
        assert_eq!(
            statuses(&state, &cookie, &writes).await,
            [StatusCode::OK, StatusCode::OK, StatusCode::NO_CONTENT]
        );
    }
}
