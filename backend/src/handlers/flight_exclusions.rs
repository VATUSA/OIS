//! Manually excluding a bogus flight from the flow picture (#342).
//!
//! A controller working an FCA drops one VATSIM callsign whose data is garbage — a teleporting
//! position, a mis-parsed route, a stuck ground squawk, a duplicate. The exclusion is stored against
//! the FCA's ARTCC (so a removal has an owner) and removes the flight from the map, the FCA crossing
//! lists, metering, counts and AADC demand for **every** viewer, because a bogus flight distorting a
//! metered flow is a shared problem rather than one each controller swats individually.
//!
//! Gated on the FCA page's existing `flow.fca.update` — no new permission. The ARTCC is always taken
//! from the FCA being worked, never from the client.

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::{
        permissions::{FlowFcaRead, FlowFcaUpdate},
        principal::{Actor, Principal},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{ExcludeFlightRequest, FcaBody, FlightExclusionBody, FlightExclusionsBody},
    repos::{flight_exclusions as exclusions_repo, flow as flow_repo},
    state::AppState,
};

/// TTL backstop for a manual exclusion (#342). The primary auto-clear is the callsign leaving the
/// VATSIM feed (`jobs::spawn_flight_exclusions_refresh`); this only catches a flight that never
/// cleanly departs it — a stuck ground squawk being the motivating case. Comfortably longer than a
/// typical leg, so a genuinely bogus flight stays hidden while it is polluting the picture, but
/// short enough that stale rows cannot accumulate.
const EXCLUSION_TTL_HOURS: i64 = 2;

/// The permission a manual exclusion is scoped against — the FCA page's existing write gate.
const EXCLUSION_PERMISSION: &str = "flow.fca.update";

/// Reload the exclusion cache so a write applies to the flow surfaces at once, instead of waiting
/// for `jobs::spawn_flight_exclusions_refresh`'s next poll.
async fn refresh_exclusions_cache(state: &AppState, pool: &sqlx::PgPool) -> Result<(), ApiError> {
    let by_artcc = exclusions_repo::load_all(pool).await?;
    state.flight_exclusions.store(std::sync::Arc::new(by_artcc));
    Ok(())
}

/// The FCA an exclusion route is working. Its ARTCC is the scope a manual exclusion is recorded under.
async fn load_fca(pool: &sqlx::PgPool, id: &str) -> Result<FcaBody, ApiError> {
    flow_repo::get_fca(pool, id)
        .await?
        .ok_or(ApiError::NotFound)
}

/// `404` for an unpublished (planned or archived) event FCA unless the caller holds
/// `events.plan.update` (#762) — the same answer as a missing id, so the route never confirms that a
/// hidden FCA exists. A planner preparing the event keeps listing, adding and removing exclusions on
/// it; published event FCAs and ordinary FCAs pass. Mirrors the `404` half of
/// `handlers::flow::require_event_fca_planner` (#736).
///
/// Every exclusion route runs this before the ARTCC scope check, so an out-of-scope caller can't
/// learn from a `403` that a planned FCA exists.
async fn hide_unpublished_event_fca(
    state: &AppState,
    principal: &Principal,
    fca: &FcaBody,
) -> Result<(), ApiError> {
    let unpublished_event =
        fca.event_id.is_some() && fca.event_status.as_deref() != Some("published");
    if unpublished_event
        && principal
            .permission_scope(state, "events.plan.update")
            .await?
            .is_empty()
    {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Fail closed unless the caller holds `flow.fca.update` nationally or for `artcc`.
///
/// `RequirePermission<FlowFcaUpdate>` only answers *whether* the caller holds the permission, not
/// *where* — so on its own a controller scoped to one facility could exclude a flight through
/// another facility's FCA. That matters more here than on the other FCA writes: the global surfaces
/// (`handlers::feed`, `handlers::gdp`, `traffic_from`) match on callsign alone via
/// `all_excluded_callsigns`, so one facility's removal hides the aircraft for **everyone**
/// nationally. Mirrors `handlers::airport_configs::require_edit` (#342).
async fn require_artcc_scope(
    state: &AppState,
    principal: &Principal,
    artcc: &str,
) -> Result<(), ApiError> {
    if may_edit_artcc(state, principal, artcc).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Whether `principal` may add or remove exclusions for `artcc` — the same question
/// [`require_artcc_scope`] answers, as a value the client can render from.
async fn may_edit_artcc(
    state: &AppState,
    principal: &Principal,
    artcc: &str,
) -> Result<bool, ApiError> {
    Ok(principal
        .permission_scope(state, EXCLUSION_PERMISSION)
        .await?
        .allows(Some(artcc)))
}

#[utoipa::path(
    get, path = "/api/v1/flow/fcas/{id}/exclusions", tag = "flow",
    security(("session" = ["flow.fca.read"]), ("api_key" = ["flow.fca.read"]), ("service_account" = ["flow.fca.read"])),
    params(("id" = String, Path)),
    responses((status = 200, body = FlightExclusionsBody), (status = 401), (status = 404))
)]
pub async fn list_flight_exclusions(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Actor(principal): Actor,
    Path(id): Path<String>,
) -> Result<Json<FlightExclusionsBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = load_fca(pool, &id).await?;
    hide_unpublished_event_fca(&state, &principal, &fca).await?;
    let artcc = fca.artcc;
    // Read is gated on `flow.fca.read`, so a viewer without any write grant still gets the list —
    // they just get `editable: false` with it.
    let editable = may_edit_artcc(&state, &principal, &artcc).await?;
    Ok(Json(FlightExclusionsBody {
        editable,
        exclusions: exclusions_repo::list_by_artcc(pool, &artcc).await?,
    }))
}

#[utoipa::path(
    post, path = "/api/v1/flow/fcas/{id}/exclusions/{callsign}", tag = "flow",
    security(("session" = ["flow.fca.update"]), ("api_key" = ["flow.fca.update"]), ("service_account" = ["flow.fca.update"])),
    params(("id" = String, Path), ("callsign" = String, Path)),
    request_body = ExcludeFlightRequest,
    responses((status = 200, body = FlightExclusionBody), (status = 401), (status = 403), (status = 404))
)]
pub async fn exclude_flight(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Actor(principal): Actor,
    Path((id, callsign)): Path<(String, String)>,
    Json(req): Json<ExcludeFlightRequest>,
) -> Result<Json<FlightExclusionBody>, ApiError> {
    let by = principal.attribution(&state).await?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let callsign = callsign.trim().to_ascii_uppercase();
    if callsign.is_empty() {
        return Err(ApiError::BadRequest);
    }
    let fca = load_fca(pool, &id).await?;
    hide_unpublished_event_fca(&state, &principal, &fca).await?;
    let artcc = fca.artcc;
    require_artcc_scope(&state, &principal, &artcc).await?;
    let row = exclusions_repo::upsert(
        pool,
        &artcc,
        &callsign,
        req.reason.trim(),
        EXCLUSION_TTL_HOURS,
        &by,
    )
    .await?;
    refresh_exclusions_cache(&state, pool).await?;
    state.publish(crate::realtime::topic::FCA);
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/flow/fcas/{id}/exclusions/{callsign}", tag = "flow",
    security(("session" = ["flow.fca.update"]), ("api_key" = ["flow.fca.update"]), ("service_account" = ["flow.fca.update"])),
    params(("id" = String, Path), ("callsign" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn restore_flight(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Actor(principal): Actor,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let callsign = callsign.trim().to_ascii_uppercase();
    let fca = load_fca(pool, &id).await?;
    hide_unpublished_event_fca(&state, &principal, &fca).await?;
    let artcc = fca.artcc;
    require_artcc_scope(&state, &principal, &artcc).await?;
    if !exclusions_repo::delete(pool, &artcc, &callsign).await? {
        return Err(ApiError::NotFound);
    }
    refresh_exclusions_cache(&state, pool).await?;
    state.publish(crate::realtime::topic::FCA);
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Scope tests (#342). A manual exclusion hides an aircraft for **every** viewer nationally, so
/// holding `flow.fca.update` somewhere must not let a controller remove a flight through another
/// facility's FCA. Mirrors `handlers::airport_configs`'s scope tests.
#[cfg(test)]
mod scope_tests {
    use sqlx::PgPool;

    use super::{may_edit_artcc, require_artcc_scope};
    use crate::scope_test_support::{self, artcc, grant, principal_for, test_state};

    fn state_with_zdc(pool: PgPool) -> crate::state::AppState {
        test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        )
    }

    #[sqlx::test]
    async fn a_national_grant_can_exclude_for_any_facility(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", None).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        assert!(require_artcc_scope(&state, &principal, "ZDC").await.is_ok());
        assert!(require_artcc_scope(&state, &principal, "ZNY").await.is_ok());
    }

    #[sqlx::test]
    async fn a_facility_grant_can_exclude_for_its_own_artcc(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        assert!(require_artcc_scope(&state, &principal, "ZDC").await.is_ok());
    }

    /// The regression this guards: `RequirePermission<FlowFcaUpdate>` alone answers "holds it", not
    /// "holds it here", so without the scope check a ZDC controller could hide an aircraft from
    /// ZNY's picture — and from everyone else's, since the global surfaces match on callsign alone.
    #[sqlx::test]
    async fn a_facility_grant_cannot_exclude_through_another_facilitys_fca(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        let err = require_artcc_scope(&state, &principal, "ZNY")
            .await
            .expect_err("a ZDC-scoped grant must not reach ZNY's FCA");
        assert!(
            matches!(err, crate::errors::ApiError::Forbidden),
            "expected 403 Forbidden, got {err:?}"
        );
    }

    /// The flag the UI renders the ✕ from must agree with the gate that answers the request —
    /// otherwise the client offers a control the server refuses with 403 (#342 rework). They share
    /// `may_edit_artcc` precisely so they cannot drift; this pins that they do.
    #[sqlx::test]
    async fn the_reported_editable_flag_matches_the_gate(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);

        for artcc in ["ZDC", "ZNY"] {
            let editable = may_edit_artcc(&state, &principal, artcc).await.unwrap();
            let gated = require_artcc_scope(&state, &principal, artcc).await.is_ok();
            assert_eq!(
                editable, gated,
                "{artcc}: editable={editable} but the write gate says {gated} — the UI would \
                 offer a control the server refuses"
            );
        }
    }

    /// A facility-scoped controller viewing another facility's FCA must be told they cannot edit,
    /// so the ✕ never renders for them in the first place.
    #[sqlx::test]
    async fn another_facilitys_fca_reports_not_editable(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        assert!(may_edit_artcc(&state, &principal, "ZDC").await.unwrap());
        assert!(!may_edit_artcc(&state, &principal, "ZNY").await.unwrap());
    }

    #[sqlx::test]
    async fn no_grant_at_all_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        assert!(
            require_artcc_scope(&state, &principal, "ZDC")
                .await
                .is_err()
        );
    }

    // --- Through the router (VATUSA/OIS#364) ---
    //
    // Everything above tests the helpers, which stay green when a handler stops calling them. These
    // send real requests with a real session, so the handler's `RequirePermission<FlowFcaUpdate>`
    // and its `require_artcc_scope` call are both on the tested path. The two gates answer
    // differently — no permission is 401, the wrong facility 403 — so each test pins one of them.

    use http::{Method, StatusCode};
    use scope_test_support::{send, send_json, session_cookie};

    const EXCLUSION: &str = "/api/v1/flow/fcas/f-zdc/exclusions/AAL1";

    /// ZDC's FCA, and a session for `user`.
    async fn routed(pool: PgPool, user: &str) -> (crate::state::AppState, String) {
        sqlx::query("insert into flow.fca (id, name, artcc) values ('f-zdc', 'ZDC FCA', 'ZDC')")
            .execute(&pool)
            .await
            .unwrap();
        let cookie = session_cookie(&pool, user).await;
        (state_with_zdc(pool), cookie)
    }

    fn reason() -> Option<serde_json::Value> {
        Some(serde_json::json!({ "reason": "ghost track" }))
    }

    /// Fails if either write drops `RequirePermission`: the scope check would then answer 403.
    #[sqlx::test]
    async fn through_the_router_a_caller_without_the_permission_gets_401(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let (state, cookie) = routed(pool, &user).await;
        let exclude = send(&state, Method::POST, EXCLUSION, &cookie, reason()).await;
        let restore = send(&state, Method::DELETE, EXCLUSION, &cookie, None).await;
        assert_eq!(exclude, StatusCode::UNAUTHORIZED, "exclude");
        assert_eq!(restore, StatusCode::UNAUTHORIZED, "restore");
    }

    /// Fails if either write drops `require_artcc_scope`: the request would then go through.
    #[sqlx::test]
    async fn through_the_router_another_facilitys_grant_gets_403(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZNY")).await;
        let (state, cookie) = routed(pool, &user).await;
        let exclude = send(&state, Method::POST, EXCLUSION, &cookie, reason()).await;
        let restore = send(&state, Method::DELETE, EXCLUSION, &cookie, None).await;
        assert_eq!(exclude, StatusCode::FORBIDDEN, "exclude");
        assert_eq!(restore, StatusCode::FORBIDDEN, "restore");
    }

    /// The positive control: the same requests go through for ZDC's own controller, so the refusals
    /// above are the gates answering and not a broken fixture.
    #[sqlx::test]
    async fn through_the_router_the_owning_facilitys_grant_goes_through(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
        let (state, cookie) = routed(pool, &user).await;
        let exclude = send(&state, Method::POST, EXCLUSION, &cookie, reason()).await;
        let restore = send(&state, Method::DELETE, EXCLUSION, &cookie, None).await;
        assert_eq!(exclude, StatusCode::OK, "exclude");
        assert_eq!(restore, StatusCode::NO_CONTENT, "restore");
    }

    // --- Unpublished event FCAs (VATUSA/OIS#762) ---
    //
    // A planned or archived event FCA answers every exclusion route with the same `404` as a missing
    // id unless the caller holds `events.plan.update` — checked before the ARTCC scope, so an
    // out-of-scope caller can't tell from a `403` that it exists. Published event FCAs and ordinary
    // FCAs answer as before.

    /// ZDC's event 7620 with one planned, one published and one archived FCA, an ordinary ZDC FCA
    /// (`f-zdc`), and a live ZDC exclusion of `AAL1` that a refused `DELETE` must leave in place.
    async fn seed_event_fcas(pool: &PgPool) {
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) values \
               (7620, 'Exclusion guard', now() + interval '1 day', now() + interval '2 days')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into flow.fca (id, name, artcc, enabled, event_id, event_status) values \
               ('ev-planned',   'Planned',   'ZDC', true, 7620, 'planned'), \
               ('ev-published', 'Published', 'ZDC', true, 7620, 'published'), \
               ('ev-archived',  'Archived',  'ZDC', true, 7620, 'archived'), \
               ('f-zdc',        'ZDC FCA',   'ZDC', true, null, null)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into flow.manual_flight_exclusion (callsign, artcc, reason, expires_at) \
             values ('AAL1', 'ZDC', 'ghost track', now() + interval '2 hours')",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Every exclusion row as `(artcc, callsign, reason)`, to prove a refused request changed nothing.
    async fn exclusion_rows(pool: &PgPool) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "select artcc, callsign, reason from flow.manual_flight_exclusion \
             order by artcc, callsign",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// A user holding each of `grants` at `artcc`, as a session cookie.
    async fn caller(pool: &PgPool, grants: &[(&str, &str)]) -> String {
        let user = scope_test_support::seed_user(pool).await;
        for (permission, artcc) in grants {
            grant(pool, &user, permission, Some(artcc)).await;
        }
        session_cookie(pool, &user).await
    }

    /// One request through the real router, answering its status and raw body — so a refusal can be
    /// compared byte for byte with a missing id's, not just by status.
    async fn call(
        state: &crate::state::AppState,
        method: Method,
        uri: &str,
        cookie: &str,
        json: Option<serde_json::Value>,
    ) -> (StatusCode, Vec<u8>) {
        use tower::ServiceExt;
        let builder = http::Request::builder()
            .method(method)
            .uri(uri)
            .header(http::header::COOKIE, cookie);
        let request = match json {
            Some(body) => builder
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(body.to_string())),
            None => builder.body(axum::body::Body::empty()),
        }
        .unwrap();
        let response = crate::router::build_router(state.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, body)
    }

    /// The three exclusion routes against FCA `id`: list, exclude `UAL2` (not yet excluded, so a
    /// refusal that leaked would add a row) and restore `AAL1` (excluded, so one would remove it).
    async fn all_three(
        state: &crate::state::AppState,
        id: &str,
        cookie: &str,
    ) -> [(&'static str, (StatusCode, Vec<u8>)); 3] {
        let list = format!("/api/v1/flow/fcas/{id}/exclusions");
        let exclude = format!("/api/v1/flow/fcas/{id}/exclusions/UAL2");
        let restore = format!("/api/v1/flow/fcas/{id}/exclusions/AAL1");
        [
            ("GET", call(state, Method::GET, &list, cookie, None).await),
            (
                "POST",
                call(state, Method::POST, &exclude, cookie, reason()).await,
            ),
            (
                "DELETE",
                call(state, Method::DELETE, &restore, cookie, None).await,
            ),
        ]
    }

    /// The acceptance case: a controller scoped elsewhere gets, on every route, exactly the `404` a
    /// missing id gets for a planned and an archived event FCA — not the `403` (writes) or `200`
    /// (list) that would confirm it exists. Nothing changes. Fails if any route drops the guard or
    /// runs it after the scope check.
    #[sqlx::test]
    async fn an_out_of_scope_caller_gets_the_missing_id_404_for_an_unpublished_event_fca(
        pool: PgPool,
    ) {
        seed_event_fcas(&pool).await;
        let cookie = caller(
            &pool,
            &[("flow.fca.read", "ZNY"), ("flow.fca.update", "ZNY")],
        )
        .await;
        let state = state_with_zdc(pool.clone());
        let before = exclusion_rows(&pool).await;

        let missing = all_three(&state, "no-such-fca", &cookie).await;
        for (route, answer) in &missing {
            assert_eq!(answer.0, StatusCode::NOT_FOUND, "{route} on a missing id");
        }
        for id in ["ev-planned", "ev-archived"] {
            for ((route, answer), (_, missing)) in
                all_three(&state, id, &cookie).await.iter().zip(&missing)
            {
                assert_eq!(answer, missing, "{route} {id} must answer as a missing id");
            }
        }
        assert_eq!(exclusion_rows(&pool).await, before, "nothing changed");

        // Positive control: a published event FCA and an ordinary one answer this caller as before —
        // listed (read is national), and refused the writes by the ARTCC scope with a 403.
        for id in ["ev-published", "f-zdc"] {
            let statuses = all_three(&state, id, &cookie)
                .await
                .map(|(r, (s, _))| (r, s));
            assert_eq!(
                statuses,
                [
                    ("GET", StatusCode::OK),
                    ("POST", StatusCode::FORBIDDEN),
                    ("DELETE", StatusCode::FORBIDDEN),
                ],
                "{id}"
            );
        }
        assert_eq!(exclusion_rows(&pool).await, before, "nothing changed");
    }

    /// The owning facility's own controller is no planner either: on ZDC's planned and archived event
    /// FCAs every route answers `404` and nothing changes, while ZDC's published event FCA and its
    /// ordinary FCA still list, exclude and restore as before.
    #[sqlx::test]
    async fn an_in_scope_non_planner_gets_404_for_an_unpublished_event_fca(pool: PgPool) {
        seed_event_fcas(&pool).await;
        let cookie = caller(
            &pool,
            &[("flow.fca.read", "ZDC"), ("flow.fca.update", "ZDC")],
        )
        .await;
        let state = state_with_zdc(pool.clone());
        let before = exclusion_rows(&pool).await;

        let missing = all_three(&state, "no-such-fca", &cookie).await;
        for id in ["ev-planned", "ev-archived"] {
            for ((route, answer), (_, missing)) in
                all_three(&state, id, &cookie).await.iter().zip(&missing)
            {
                assert_eq!(answer.0, StatusCode::NOT_FOUND, "{route} {id}");
                assert_eq!(answer, missing, "{route} {id} must answer as a missing id");
            }
        }
        assert_eq!(exclusion_rows(&pool).await, before, "nothing changed");

        // Positive control: unchanged for a published event FCA and an ordinary one.
        for id in ["ev-published", "f-zdc"] {
            let statuses = all_three(&state, id, &cookie)
                .await
                .map(|(r, (s, _))| (r, s));
            assert_eq!(
                statuses,
                [
                    ("GET", StatusCode::OK),
                    ("POST", StatusCode::OK),
                    ("DELETE", StatusCode::NO_CONTENT),
                ],
                "{id}"
            );
            // Put AAL1 back for the next FCA's restore, and drop UAL2 so its exclude adds it again.
            sqlx::query(
                "insert into flow.manual_flight_exclusion (callsign, artcc, reason, expires_at) \
                 values ('AAL1', 'ZDC', 'ghost track', now() + interval '2 hours')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("delete from flow.manual_flight_exclusion where callsign = 'UAL2'")
                .execute(&pool)
                .await
                .unwrap();
        }
    }

    /// A planner preparing the event keeps all three routes on a planned and an archived event FCA —
    /// and the ARTCC scope check still runs after the guard, so a planner whose `flow.fca.update` is
    /// elsewhere is refused the writes with the usual `403`.
    #[sqlx::test]
    async fn a_planner_lists_adds_and_removes_on_an_unpublished_event_fca(pool: PgPool) {
        seed_event_fcas(&pool).await;
        let planner = caller(
            &pool,
            &[
                ("events.plan.update", "ZDC"),
                ("flow.fca.read", "ZDC"),
                ("flow.fca.update", "ZDC"),
            ],
        )
        .await;
        let elsewhere = caller(
            &pool,
            &[
                ("events.plan.update", "ZDC"),
                ("flow.fca.read", "ZNY"),
                ("flow.fca.update", "ZNY"),
            ],
        )
        .await;
        let state = state_with_zdc(pool.clone());

        for id in ["ev-planned", "ev-archived"] {
            let (status, body) = send_json(
                &state,
                Method::GET,
                &format!("/api/v1/flow/fcas/{id}/exclusions"),
                &planner,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "GET {id}");
            assert_eq!(body["editable"], true, "GET {id}: {body}");
            assert!(
                body.to_string().contains("AAL1"),
                "GET {id} lists AAL1: {body}"
            );

            let excluded = format!("/api/v1/flow/fcas/{id}/exclusions/UAL2");
            assert_eq!(
                send(&state, Method::POST, &excluded, &planner, reason()).await,
                StatusCode::OK,
                "POST {id}"
            );
            assert!(
                exclusion_rows(&pool)
                    .await
                    .iter()
                    .any(|(_, c, _)| c == "UAL2"),
                "POST {id} added UAL2"
            );
            assert_eq!(
                send(&state, Method::DELETE, &excluded, &planner, None).await,
                StatusCode::NO_CONTENT,
                "DELETE {id}"
            );
            assert!(
                !exclusion_rows(&pool)
                    .await
                    .iter()
                    .any(|(_, c, _)| c == "UAL2"),
                "DELETE {id} removed UAL2"
            );

            let before = exclusion_rows(&pool).await;
            let statuses = all_three(&state, id, &elsewhere)
                .await
                .map(|(r, (s, _))| (r, s));
            assert_eq!(
                statuses,
                [
                    ("GET", StatusCode::OK),
                    ("POST", StatusCode::FORBIDDEN),
                    ("DELETE", StatusCode::FORBIDDEN),
                ],
                "{id}, planner scoped elsewhere for flow.fca.update"
            );
            assert_eq!(exclusion_rows(&pool).await, before, "nothing changed");
        }
    }
}
