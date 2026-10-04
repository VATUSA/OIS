//! Discord integration endpoints: the outbound-job queue the bot drains (lease/ack, gated
//! `integration.jobs.update`), and the guild config the operators edit (`discord.config.*`).

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    auth::{
        context::{CurrentServiceAccount, CurrentUser},
        permissions::{DiscordConfigRead, DiscordConfigUpdate, IntegrationJobsUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    handlers::events::normalize_facility,
    models::{
        AceRequestBody, AckJobRequest, DiscordAceClaimRequest, DiscordAceInfoBody,
        DiscordAdvisoryInfoBody, DiscordAvailabilityRequest, DiscordAvailabilityResult,
        DiscordConfigBody, DiscordLinkBody, DiscordTmiInfoBody, EventThreadTemplateBody,
        OutboundJobBody, PushGuildSnapshotRequest, UpsertDiscordConfigRequest,
        UpsertEventThreadTemplateRequest,
    },
    repos::{
        access as access_repo, ace as ace_repo, availability as availability_repo,
        events as events_repo, integration as integration_repo, org as org_repo, tmu as tmu_repo,
    },
    state::AppState,
};

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

/// The queue consumer a lease or ack acts for: the calling **service account's key** (#656). A queue
/// consumer is a machine, so a person's session or API key is refused; a declared `?consumer=` is
/// optional and must name that same key. Before this the consumer was whatever the caller declared, so
/// any `integration.jobs.update` holder could lease or ack as the Discord bot (#590 stopped only the
/// accidental case).
fn bound_consumer<'a>(
    account: Option<&'a CurrentServiceAccount>,
    declared: Option<&str>,
) -> Result<&'a str, ApiError> {
    let account = account.ok_or(ApiError::Forbidden)?;
    match declared.map(str::trim).filter(|c| !c.is_empty()) {
        Some(declared) if declared != account.key => Err(ApiError::Forbidden),
        _ => Ok(&account.key),
    }
}

#[derive(Deserialize)]
pub struct LeaseQuery {
    /// Optional; must be the caller's own consumer — see [`bound_consumer`].
    consumer: Option<String>,
    /// Max jobs to lease (default 10, clamped 1–100).
    limit: Option<i64>,
}

#[utoipa::path(
    post, path = "/api/v1/integration/jobs/lease", tag = "integration",
    params(
        ("consumer" = Option<String>, Query, description = "Optional. The consumer is the calling service account's key (the Discord bot's is `discord`); if given, this must match it."),
        ("limit" = Option<i64>, Query, description = "Max jobs (default 10)")
    ),
    responses(
        (status = 200, body = Vec<OutboundJobBody>),
        (status = 401),
        (status = 403, description = "Not a service account, or `consumer` names another consumer")
    )
)]
pub async fn lease_jobs(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Extension(account): Extension<Option<CurrentServiceAccount>>,
    Query(q): Query<LeaseQuery>,
) -> Result<Json<Vec<OutboundJobBody>>, ApiError> {
    let consumer = bound_consumer(account.as_ref(), q.consumer.as_deref())?;
    let limit = q.limit.unwrap_or(10);
    Ok(Json(
        integration_repo::lease_jobs(pool(&state)?, consumer, limit).await?,
    ))
}

#[derive(Deserialize)]
pub struct AckQuery {
    /// Optional; must be the caller's own consumer — see [`bound_consumer`]. An ack applies only to that
    /// consumer's job.
    consumer: Option<String>,
}

#[utoipa::path(
    post, path = "/api/v1/integration/jobs/{id}/ack", tag = "integration",
    params(
        ("id" = String, Path),
        ("consumer" = Option<String>, Query, description = "Optional. The consumer is the calling service account's key; if given, this must match it. An ack applies only to that consumer's job.")
    ),
    request_body = AckJobRequest,
    responses(
        (status = 204), (status = 401),
        (status = 403, description = "Not a service account, or `consumer` names another consumer"),
        (status = 404)
    )
)]
pub async fn ack_job(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Extension(account): Extension<Option<CurrentServiceAccount>>,
    Path(id): Path<String>,
    Query(q): Query<AckQuery>,
    Json(payload): Json<AckJobRequest>,
) -> Result<StatusCode, ApiError> {
    let consumer = bound_consumer(account.as_ref(), q.consumer.as_deref())?;
    let ok = integration_repo::ack_job(
        pool(&state)?,
        &id,
        consumer,
        payload.success,
        payload.result.as_ref(),
        payload.error.as_deref(),
        payload.attempt,
    )
    .await?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Either the id is unknown, or the ack is stale — the job was reaped for running past its
        // lease and re-leased, so this worker no longer owns it (#446 review). Both answer 404 to the
        // bot, which retries nothing; the log is what makes the second case diagnosable, because a
        // stale ack means a worker took longer than OUTBOUND_JOB_LEASE_TIMEOUT_MINS and its Discord
        // call may well have gone out twice.
        tracing::warn!(
            job = %id,
            success = payload.success,
            attempt = ?payload.attempt,
            consumer,
            "integration: ack did not apply — unknown job, another consumer's job, or this worker no longer holds the lease"
        );
        Err(ApiError::NotFound)
    }
}

// --- current user's Discord link (read-only; sourced from VATUSA) ---

#[utoipa::path(
    get, path = "/api/v1/me/discord", tag = "integration",
    responses((status = 200, body = DiscordLinkBody), (status = 401))
)]
pub async fn get_my_discord(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<Json<DiscordLinkBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let link = integration_repo::get_discord_link(pool(&state)?, &user.id).await?;
    Ok(Json(match link {
        Some((discord_id, _meta)) => DiscordLinkBody {
            linked: true,
            discord_id: Some(discord_id),
            // Username isn't provided by VATUSA — only the id.
            username: None,
        },
        None => DiscordLinkBody {
            linked: false,
            discord_id: None,
            username: None,
        },
    }))
}

// --- interaction callbacks (bot acts on behalf of the linked user) ---

#[utoipa::path(
    get, path = "/api/v1/integration/discord/ace/{id}", tag = "integration",
    params(("id" = String, Path)),
    responses((status = 200, body = DiscordAceInfoBody), (status = 401), (status = 404))
)]
pub async fn discord_ace_info(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<String>,
) -> Result<Json<DiscordAceInfoBody>, ApiError> {
    let p = pool(&state)?;
    let request = ace_repo::get_request(p, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let event = events_repo::get(p, request.event_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (window_label, time_options) =
        crate::handlers::ace::event_time_options(event.start_time, event.end_time);
    Ok(Json(DiscordAceInfoBody {
        event_title: event.title,
        window_label,
        slots: request.slots,
        claims_count: request.claims_count,
        time_options,
    }))
}

/// What the bot needs to reply to a "View structured" button click on a TMI post.
#[utoipa::path(
    get, path = "/api/v1/integration/discord/tmi/{id}", tag = "integration",
    params(("id" = String, Path)),
    responses((status = 200, body = DiscordTmiInfoBody), (status = 401), (status = 404))
)]
pub async fn discord_tmi_info(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<String>,
) -> Result<Json<DiscordTmiInfoBody>, ApiError> {
    let tmi = tmu_repo::get_tmi(pool(&state)?, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(DiscordTmiInfoBody {
        restriction: tmi.restriction,
        decoded: tmi.decoded,
    }))
}

/// What the bot needs to reply to a "View structured" button click on an advisory post.
#[utoipa::path(
    get, path = "/api/v1/integration/discord/advisory/{id}", tag = "integration",
    params(("id" = String, Path)),
    responses((status = 200, body = DiscordAdvisoryInfoBody), (status = 401), (status = 404))
)]
pub async fn discord_advisory_info(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<String>,
) -> Result<Json<DiscordAdvisoryInfoBody>, ApiError> {
    let adv = tmu_repo::get_advisory(pool(&state)?, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(DiscordAdvisoryInfoBody {
        kind: adv.kind,
        structured: adv.structured.map(|j| j.0),
    }))
}

#[utoipa::path(
    post, path = "/api/v1/integration/discord/ace/{id}/claim", tag = "integration",
    params(("id" = String, Path)), request_body = DiscordAceClaimRequest,
    responses(
        (status = 200, body = AceRequestBody), (status = 401),
        (status = 403, description = "Discord account not linked to an OIS user"),
        (status = 404), (status = 409)
    )
)]
pub async fn discord_ace_claim(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<String>,
    Json(payload): Json<DiscordAceClaimRequest>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let p = pool(&state)?;
    // The clicking Discord user must have linked their OIS account — that's who the claim belongs to.
    let user_id = integration_repo::find_user_by_discord_id(p, &payload.discord_user_id)
        .await?
        .ok_or(ApiError::Forbidden)?;

    // Parse the modal's Zulu HHMM against the request's event window (the bot has no per-message state).
    let request = ace_repo::get_request(p, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let event = events_repo::get(p, request.event_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let start = crate::handlers::ace::parse_hhmm_in_window(
        payload.start_hhmm.as_deref(),
        event.start_time,
        event.end_time,
    );
    let end = crate::handlers::ace::parse_hhmm_in_window(
        payload.end_hhmm.as_deref(),
        event.start_time,
        event.end_time,
    );
    let notes = payload.notes.as_deref().unwrap_or("").trim().to_string();

    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    let (slots, count) =
        ace_repo::claim_request(&mut tx, &id, &user_id, &notes, start, end).await?;
    crate::handlers::ace::enqueue_notify(&mut tx, p, &id, slots, count).await?;
    crate::handlers::ace::enqueue_claim_dm(&mut tx, p, &id, &user_id).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    ace_repo::get_request(p, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Availability button (🟢/🟡/🔴) on a DCC event thread. The bot relays the click; we resolve the
/// Discord user to their linked OIS account and record the response — but only if they actually hold
/// `events.availability.update` (NTMOs / DCC staff). Refusals come back as `ok=false` (never an error
/// status) so the bot can explain why to the user.
#[utoipa::path(
    post, path = "/api/v1/integration/discord/availability/{id}", tag = "integration",
    params(("id" = i64, Path)), request_body = DiscordAvailabilityRequest,
    responses((status = 200, body = DiscordAvailabilityResult), (status = 401), (status = 404))
)]
pub async fn discord_availability(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<i64>,
    Json(payload): Json<DiscordAvailabilityRequest>,
) -> Result<Json<DiscordAvailabilityResult>, ApiError> {
    let p = pool(&state)?;

    let refuse = |reason: &str| {
        Ok(Json(DiscordAvailabilityResult {
            ok: false,
            reason: Some(reason.to_string()),
            display_name: None,
            status: None,
        }))
    };

    // Only the three known button values are accepted.
    if !matches!(
        payload.status.as_str(),
        "available" | "partial" | "unavailable"
    ) {
        return refuse("invalid");
    }

    // The event must still exist (threads can outlive pruned events).
    events_repo::get(p, id).await?.ok_or(ApiError::NotFound)?;

    // Resolve the presser to an OIS user (VATUSA-linked Discord); unlinked users can't respond.
    let Some(user_id) =
        integration_repo::find_user_by_discord_id(p, &payload.discord_user_id).await?
    else {
        return refuse("unlinked");
    };

    // Gate on the resolved user's effective permissions — NTMO / DCC staff by default.
    let perms = access_repo::fetch_user_permission_names(p, &user_id).await?;
    if !perms
        .iter()
        .any(|perm| perm == "events.availability.update")
    {
        return refuse("forbidden");
    }

    availability_repo::set_availability(p, id, &user_id, &payload.status).await?;
    state.publish(crate::realtime::topic::EVENT_AVAILABILITY);
    let display_name = access_repo::user_display_name(p, &user_id).await?;
    Ok(Json(DiscordAvailabilityResult {
        ok: true,
        reason: None,
        display_name,
        status: Some(payload.status),
    }))
}

// --- guild config ---

#[utoipa::path(
    get, path = "/api/v1/integration/discord", tag = "integration",
    responses((status = 200, body = DiscordConfigBody), (status = 401))
)]
pub async fn get_discord_config(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigRead>,
) -> Result<Json<DiscordConfigBody>, ApiError> {
    Ok(Json(integration_repo::get_config(pool(&state)?).await?))
}

#[utoipa::path(
    put, path = "/api/v1/integration/discord", tag = "integration",
    request_body = UpsertDiscordConfigRequest,
    responses((status = 200, body = DiscordConfigBody), (status = 400), (status = 401))
)]
pub async fn put_discord_config(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigUpdate>,
    Json(payload): Json<UpsertDiscordConfigRequest>,
) -> Result<Json<DiscordConfigBody>, ApiError> {
    let p = pool(&state)?;
    for g in &payload.guilds {
        if g.name.trim().is_empty() || g.guild_id.trim().is_empty() {
            return Err(ApiError::BadRequest);
        }
        // Reject rather than silently drop: an invalid facility here would otherwise vanish with a
        // 200 response, leaving no signal to the caller that the value they submitted never made it
        // into discord_config_facilities (#194). Checks existence, not just shape — a well-formed
        // but nonexistent/typo'd code (e.g. "ZDX") would otherwise pass this check and only fail
        // later as an opaque 500 from discord_config_facilities' FK constraint (migration 0070).
        for f in &g.facilities {
            let Some(id) = normalize_facility(f) else {
                return Err(ApiError::BadRequest);
            };
            if org_repo::find_facility(p, &id).await?.is_none() {
                return Err(ApiError::BadRequest);
            }
        }
    }
    integration_repo::upsert_config(p, &payload).await?;
    Ok(Json(integration_repo::get_config(p).await?))
}

#[utoipa::path(
    get, path = "/api/v1/integration/discord/thread-template", tag = "integration",
    responses((status = 200, body = EventThreadTemplateBody), (status = 401))
)]
pub async fn get_event_thread_template(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigRead>,
) -> Result<Json<EventThreadTemplateBody>, ApiError> {
    let body = integration_repo::get_event_thread_template(pool(&state)?).await?;
    Ok(Json(EventThreadTemplateBody { body }))
}

/// Discord rejects a message over 2000 chars outright. Cap well under that so the per-event
/// substitutions (title, date_line, facility_lines — one line per required/preferred facility) have
/// headroom before the bot's own truncation safety net (`jobs::thread::create_event_thread`) has to
/// kick in. Counted in `char`s (Unicode scalar values), not bytes — the shipped default template
/// itself uses multi-byte box-drawing dividers and emoji, so a byte-length check would reject a
/// template that looks well under the cap in the textarea the admin is actually looking at.
const MAX_TEMPLATE_LEN: usize = 1500;

fn validate_template_body(body: &str) -> Result<&str, ApiError> {
    let trimmed = body.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_TEMPLATE_LEN {
        return Err(ApiError::BadRequest);
    }
    Ok(trimmed)
}

#[utoipa::path(
    put, path = "/api/v1/integration/discord/thread-template", tag = "integration",
    request_body = UpsertEventThreadTemplateRequest,
    responses((status = 200, body = EventThreadTemplateBody), (status = 400), (status = 401))
)]
pub async fn put_event_thread_template(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigUpdate>,
    Json(payload): Json<UpsertEventThreadTemplateRequest>,
) -> Result<Json<EventThreadTemplateBody>, ApiError> {
    let trimmed = validate_template_body(&payload.body)?;
    let body = integration_repo::set_event_thread_template(pool(&state)?, trimmed).await?;
    Ok(Json(EventThreadTemplateBody { body }))
}

#[cfg(test)]
mod thread_template_tests {
    use super::*;

    #[test]
    fn rejects_empty_or_whitespace_only_body() {
        assert!(matches!(
            validate_template_body(""),
            Err(ApiError::BadRequest)
        ));
        assert!(matches!(
            validate_template_body("   \n  "),
            Err(ApiError::BadRequest)
        ));
    }

    #[test]
    fn rejects_a_body_over_the_max_length() {
        let too_long = "a".repeat(MAX_TEMPLATE_LEN + 1);
        assert!(matches!(
            validate_template_body(&too_long),
            Err(ApiError::BadRequest)
        ));
    }

    #[test]
    fn accepts_a_body_at_or_under_the_max_length() {
        let at_max = "a".repeat(MAX_TEMPLATE_LEN);
        assert_eq!(
            validate_template_body(&at_max).unwrap().chars().count(),
            MAX_TEMPLATE_LEN
        );
        assert_eq!(validate_template_body("  hi  ").unwrap(), "hi");
    }

    /// The cap counts characters, not bytes — a template built from the same multi-byte box-drawing
    /// dividers and emoji as the shipped default must not be rejected just because its byte length
    /// exceeds the char cap while its actual character count doesn't.
    #[test]
    fn multi_byte_characters_are_counted_once_each_not_by_their_byte_length() {
        // "─" is 3 bytes and "🟢" is 4 bytes in UTF-8, so this string's byte length is well over
        // MAX_TEMPLATE_LEN even though its character count is far under it.
        let body: String = "─🟢".repeat(300);
        assert!(body.len() > MAX_TEMPLATE_LEN); // sanity check: byte length would wrongly reject this
        assert!(body.chars().count() < MAX_TEMPLATE_LEN);
        assert!(validate_template_body(&body).is_ok());
    }
}

/// The bot pushes the guilds it's in (channels + roles) so the editor can offer dropdowns. Gated by
/// the bot's `integration.jobs.update` (a human admin never calls this).
#[utoipa::path(
    post, path = "/api/v1/integration/discord/guilds/snapshot", tag = "integration",
    request_body = PushGuildSnapshotRequest,
    responses((status = 204), (status = 401))
)]
pub async fn push_guild_snapshot(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Json(payload): Json<PushGuildSnapshotRequest>,
) -> Result<StatusCode, ApiError> {
    integration_repo::replace_guild_snapshots(pool(&state)?, &payload.guilds).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ask the bot to re-pull the guild snapshot (the admin's "Refresh from Discord" button). Enqueues a
/// `guild_snapshot` job the bot handles by pushing a fresh snapshot. Gated by `discord.config.update`.
#[utoipa::path(
    post, path = "/api/v1/integration/discord/refresh", tag = "integration",
    responses((status = 202), (status = 401))
)]
pub async fn refresh_guild_snapshot(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigUpdate>,
) -> Result<StatusCode, ApiError> {
    let p = pool(&state)?;
    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    integration_repo::enqueue_job(
        &mut tx,
        "guild_snapshot",
        &serde_json::json!({}),
        None,
        None,
    )
    .await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use crate::scope_test_support::{grant, seed_user, session_cookie, test_state};

    /// A queue consumer as the bot is one (#656): a service account keyed `key`, as its bearer. With
    /// `bot_role` it holds `BOT`, which grants `integration.jobs.update`.
    async fn account(pool: &PgPool, key: &str, bot_role: bool) -> (String, String) {
        let id: String = sqlx::query_scalar(
            "insert into access.service_accounts (key, name) values ($1, $1) returning id",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap();
        let token = format!("ois_sa_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(
            "insert into access.service_account_credentials (service_account_id, secret_hash) \
             values ($1, $2)",
        )
        .bind(&id)
        .bind(crate::repos::access::sha256_hex(&token))
        .execute(pool)
        .await
        .unwrap();
        if bot_role {
            give_bot_role(pool, &id).await;
        }
        (id, format!("Bearer {token}"))
    }

    async fn give_bot_role(pool: &PgPool, account_id: &str) {
        sqlx::query(
            "insert into access.service_account_roles (service_account_id, role_name) \
             values ($1, 'BOT')",
        )
        .bind(account_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// The bearer of a `BOT` service account keyed `key`.
    async fn consumer(pool: &PgPool, key: &str) -> String {
        account(pool, key, true).await.1
    }

    /// One request through the real router with `credential` as a bearer, or as the session cookie.
    async fn call(
        state: &crate::state::AppState,
        method: http::Method,
        uri: &str,
        credential: &str,
        json: Option<serde_json::Value>,
    ) -> http::StatusCode {
        use tower::ServiceExt;

        let header = if credential.starts_with("Bearer ") {
            http::header::AUTHORIZATION
        } else {
            http::header::COOKIE
        };
        let builder = http::Request::builder()
            .method(method)
            .uri(uri)
            .header(header, credential);
        let request = match json {
            Some(body) => builder
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(body.to_string())),
            None => builder.body(axum::body::Body::empty()),
        }
        .unwrap();
        crate::router::build_router(state.clone())
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }

    /// A job `in_progress` on `attempt_count`, as a successor holds it after a reap and re-lease.
    ///
    /// Written directly rather than driven through lease → reap → re-lease: that sequence is already
    /// covered in `repos::integration::tests::a_predecessors_ack_cannot_touch_a_re_leased_job`, and
    /// what these tests are about is the route, not how the row reached this state.
    async fn job_held_on_attempt(pool: &PgPool, attempt_count: i32) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into integration.outbound_jobs (job_type, status, attempt_count, last_attempt_at) \
             values ('tmi_publish', 'in_progress', $1, now()) returning id",
        )
        .bind(attempt_count)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn status_of(pool: &PgPool, id: &str) -> String {
        sqlx::query_scalar::<_, String>(
            "select status from integration.outbound_jobs where id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The lease fence has to survive the trip through the handler, and nothing else proves it does.
    ///
    /// `repos::integration::ack_job`'s own tests call the function directly, so replacing
    /// `payload.attempt` with `None` here — which silently reduces every ack to the status-only fence
    /// and makes VATUSA/OIS#472's fix a no-op in production — left all 612 of them green. This drives
    /// the real route instead (`scope_test_support::send`, VATUSA/OIS#364), so the handler is on the
    /// tested path.
    ///
    /// It also pins the field's name on the wire. `attempt` is `#[serde(default)]` by necessity — an
    /// old bot must keep working across the deploy — so a rename doesn't fail to parse, it quietly
    /// deserialises to `None` and falls back to the fence this issue exists to replace.
    #[sqlx::test]
    async fn the_route_refuses_a_predecessors_ack_and_accepts_the_holders(pool: PgPool) {
        let id = job_held_on_attempt(&pool, 2).await;
        let cookie = consumer(&pool, "discord").await;
        let state = test_state(pool.clone(), Default::default());
        let uri = format!("/api/v1/integration/jobs/{id}/ack?consumer=discord");

        let stale = call(
            &state,
            http::Method::POST,
            &uri,
            &cookie,
            Some(serde_json::json!({"success": true, "attempt": 1})),
        )
        .await;
        assert_eq!(
            stale,
            http::StatusCode::NOT_FOUND,
            "attempt 1 no longer holds the lease, so the ack must not apply"
        );
        assert_eq!(
            status_of(&pool, &id).await,
            "in_progress",
            "the successor is still working the job"
        );

        let current = call(
            &state,
            http::Method::POST,
            &uri,
            &cookie,
            Some(serde_json::json!({"success": true, "attempt": 2})),
        )
        .await;
        assert_eq!(current, http::StatusCode::NO_CONTENT);
        assert_eq!(status_of(&pool, &id).await, "succeeded");
    }

    /// The old-bot path, through the route: no `attempt` at all must still be accepted, or deploying
    /// the backend ahead of the bot rejects every ack and turns a narrow race into total delivery
    /// failure.
    #[sqlx::test]
    async fn the_route_still_accepts_an_ack_carrying_no_lease_token(pool: PgPool) {
        let id = job_held_on_attempt(&pool, 2).await;
        let cookie = consumer(&pool, "discord").await;
        let state = test_state(pool.clone(), Default::default());

        let status = call(
            &state,
            http::Method::POST,
            &format!("/api/v1/integration/jobs/{id}/ack?consumer=discord"),
            &cookie,
            Some(serde_json::json!({"success": true})),
        )
        .await;

        assert_eq!(status, http::StatusCode::NO_CONTENT);
        assert_eq!(status_of(&pool, &id).await, "succeeded");
    }

    /// Acking a job is state-mutating, so it must be gated — and until now no test said so. A caller
    /// without `integration.jobs.update` is refused by `RequirePermission` before the handler runs,
    /// which is a **401**, distinct from the 404 a stale ack earns.
    #[sqlx::test]
    async fn acking_a_job_requires_the_integration_jobs_permission(pool: PgPool) {
        let id = job_held_on_attempt(&pool, 1).await;
        let (account_id, cookie) = account(&pool, "discord", false).await;
        let state = test_state(pool.clone(), Default::default());
        let uri = format!("/api/v1/integration/jobs/{id}/ack?consumer=discord");
        let body = serde_json::json!({"success": true, "attempt": 1});

        let refused = call(
            &state,
            http::Method::POST,
            &uri,
            &cookie,
            Some(body.clone()),
        )
        .await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            status_of(&pool, &id).await,
            "in_progress",
            "a refused ack must not have touched the row"
        );

        give_bot_role(&pool, &account_id).await;
        let allowed = call(&state, http::Method::POST, &uri, &cookie, Some(body)).await;
        assert_eq!(allowed, http::StatusCode::NO_CONTENT);
    }
    /// A Discord job made the way production makes one — through `enqueue_job` — so the test proves
    /// what the bot will actually see, not what a fixture says.
    async fn enqueued_discord_job(pool: &PgPool) -> String {
        let mut tx = pool.begin().await.unwrap();
        let id = crate::repos::integration::enqueue_job(
            &mut tx,
            "guild_snapshot",
            &serde_json::json!({}),
            None,
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        id
    }

    /// A due job belonging to another consumer — what an outbound-webhook worker would lease.
    async fn webhook_job(pool: &PgPool) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into integration.outbound_jobs (job_type, consumer) \
             values ('webhook_delivery', 'webhook') returning id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// #590 AC1: two consumers lease from the one queue and neither takes the other's job. Each is now
    /// its own service account (#656), and leases without naming itself.
    #[sqlx::test]
    async fn two_consumers_lease_only_their_own_jobs(pool: PgPool) {
        let discord = enqueued_discord_job(&pool).await;
        let webhook = webhook_job(&pool).await;
        let (bot, hook) = (
            consumer(&pool, "discord").await,
            consumer(&pool, "webhook").await,
        );
        let state = test_state(pool.clone(), Default::default());
        let lease = "/api/v1/integration/jobs/lease";

        assert_eq!(
            call(&state, http::Method::POST, lease, &hook, None).await,
            http::StatusCode::OK
        );
        assert_eq!(status_of(&pool, &webhook).await, "in_progress");
        assert_eq!(
            status_of(&pool, &discord).await,
            "pending",
            "the webhook consumer must not take the bot's job"
        );

        // AC2: the bot, leasing as `discord/src/jobs/mod.rs` does — naming itself — still gets its job.
        let ok = call(
            &state,
            http::Method::POST,
            &format!("{lease}?consumer=discord"),
            &bot,
            None,
        )
        .await;
        assert_eq!(ok, http::StatusCode::OK);
        assert_eq!(status_of(&pool, &discord).await, "in_progress");
    }

    /// #656 AC1: a consumer can't lease another's jobs by naming it. The `webhook` account asking for
    /// `discord`'s jobs is refused, and the bot's job is untouched.
    #[sqlx::test]
    async fn a_consumer_cannot_lease_as_another(pool: PgPool) {
        let discord = enqueued_discord_job(&pool).await;
        let hook = consumer(&pool, "webhook").await;
        let state = test_state(pool.clone(), Default::default());

        let refused = call(
            &state,
            http::Method::POST,
            "/api/v1/integration/jobs/lease?consumer=discord",
            &hook,
            None,
        )
        .await;

        assert_eq!(refused, http::StatusCode::FORBIDDEN);
        assert_eq!(status_of(&pool, &discord).await, "pending");
    }

    /// A queue consumer is a machine: a person holding `integration.jobs.update` — by session or API
    /// key — can't lease or ack, so they can't drain the bot's queue either.
    #[sqlx::test]
    async fn a_person_cannot_lease_or_ack(pool: PgPool) {
        let discord = enqueued_discord_job(&pool).await;
        let held = job_held_on_attempt(&pool, 1).await;
        let user = seed_user(&pool).await;
        grant(&pool, &user, "integration.jobs.update", None).await;
        let cookie = session_cookie(&pool, &user).await;
        let state = test_state(pool.clone(), Default::default());

        let lease = call(
            &state,
            http::Method::POST,
            "/api/v1/integration/jobs/lease?consumer=discord",
            &cookie,
            None,
        )
        .await;
        let ack = call(
            &state,
            http::Method::POST,
            &format!("/api/v1/integration/jobs/{held}/ack?consumer=discord"),
            &cookie,
            Some(serde_json::json!({"success": true, "attempt": 1})),
        )
        .await;

        assert_eq!(lease, http::StatusCode::FORBIDDEN);
        assert_eq!(ack, http::StatusCode::FORBIDDEN);
        assert_eq!(status_of(&pool, &discord).await, "pending");
        assert_eq!(status_of(&pool, &held).await, "in_progress");
    }

    /// #590 review + #656 AC1: an ack applies only to the caller's own consumer's job. The `webhook`
    /// account can't complete or fail the bot's in-flight job — by its id alone (404), or by also
    /// claiming to be `discord` (403).
    #[sqlx::test]
    async fn an_ack_from_another_consumer_does_not_apply(pool: PgPool) {
        let id = job_held_on_attempt(&pool, 1).await; // the bot's: consumer defaults to `discord`
        let (bot, hook) = (
            consumer(&pool, "discord").await,
            consumer(&pool, "webhook").await,
        );
        let state = test_state(pool.clone(), Default::default());
        let body = serde_json::json!({"success": true, "attempt": 1});
        let fail = serde_json::json!({"success": false, "error": "not mine", "attempt": 1});
        let ack = format!("/api/v1/integration/jobs/{id}/ack");

        for b in [body.clone(), fail] {
            let own = call(&state, http::Method::POST, &ack, &hook, Some(b.clone())).await;
            assert_eq!(
                own,
                http::StatusCode::NOT_FOUND,
                "not the webhook consumer's job"
            );
            let spoofed = call(
                &state,
                http::Method::POST,
                &format!("{ack}?consumer=discord"),
                &hook,
                Some(b),
            )
            .await;
            assert_eq!(
                spoofed,
                http::StatusCode::FORBIDDEN,
                "and it can't claim to be the bot"
            );
            assert_eq!(
                status_of(&pool, &id).await,
                "in_progress",
                "the bot's job is untouched"
            );
        }

        let applied = call(&state, http::Method::POST, &ack, &bot, Some(body)).await;
        assert_eq!(applied, http::StatusCode::NO_CONTENT);
        assert_eq!(status_of(&pool, &id).await, "succeeded");
    }
    // ---- #656 QA: the deploy renames the bot's account, never a guess ------------------------------

    /// The migration exactly as it ships, so editing `0115` can't leave these green on a stale copy.
    const BOT_RENAME: &str =
        include_str!("../../migrations/0115_bot_account_is_the_discord_consumer.sql");

    async fn rename(pool: &PgPool) {
        sqlx::raw_sql(BOT_RENAME)
            .execute(pool)
            .await
            .expect("0115 runs");
    }

    async fn keys(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar("select key from access.service_accounts order by key")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn the_one_bot_account_becomes_the_discord_consumer(pool: PgPool) {
        account(&pool, "ois-discord-bot", true).await;
        account(&pool, "vtbfm", false).await;
        rename(&pool).await;
        assert_eq!(keys(&pool).await, ["discord", "vtbfm"]);
    }

    /// Two candidates is a guess, so nothing changes, and the migration still runs.
    #[sqlx::test]
    async fn two_bot_accounts_are_left_alone(pool: PgPool) {
        account(&pool, "bot-a", true).await;
        account(&pool, "bot-b", true).await;
        rename(&pool).await;
        assert_eq!(keys(&pool).await, ["bot-a", "bot-b"]);
    }

    #[sqlx::test]
    async fn an_existing_discord_key_is_left_alone(pool: PgPool) {
        account(&pool, "discord", false).await;
        account(&pool, "ois-discord-bot", true).await;
        rename(&pool).await;
        assert_eq!(keys(&pool).await, ["discord", "ois-discord-bot"]);
    }

    /// Only a live bot counts: a disabled account or an expired BOT grant is not a candidate, so the
    /// one live bot is still renamed.
    #[sqlx::test]
    async fn a_disabled_or_expired_bot_is_not_a_candidate(pool: PgPool) {
        account(&pool, "ois-discord-bot", true).await;
        let (disabled, _) = account(&pool, "old-bot", true).await;
        sqlx::query("update access.service_accounts set status = 'disabled' where id = $1")
            .bind(&disabled)
            .execute(&pool)
            .await
            .unwrap();
        let (expired, _) = account(&pool, "lapsed-bot", true).await;
        sqlx::query(
            "update access.service_account_roles set ends_at = now() - interval '1 day' \
             where service_account_id = $1",
        )
        .bind(&expired)
        .execute(&pool)
        .await
        .unwrap();
        rename(&pool).await;
        assert_eq!(keys(&pool).await, ["discord", "lapsed-bot", "old-bot"]);
    }
}
