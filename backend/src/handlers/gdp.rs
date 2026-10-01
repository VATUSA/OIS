//! Ground Delay Program handlers — the program lifecycle (draft → published → cancelled) and
//! the live board (Ration-By-Schedule control times joined to current traffic), computed off
//! the feed. Control times are frozen into `tmu.gdp_slot` at publish so issued EDCTs hold.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{TmuGdpCreate, TmuGdpDelete, TmuGdpPublish, TmuGdpRead},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::facilities,
    feed::flow,
    feed::gdp::{self, GdpBoard, GdpFlightView},
    handlers::restriction_artcc,
    models::{
        AarStep, CreateAdvisoryRequest, CreateGdpRequest, GdpBody, PublishGdpRequest,
        UpdateGdpRequest,
    },
    repos::gdp as gdp_repo,
    repos::tmu as tmu_repo,
    state::AppState,
};

/// Parse a required HHMM Zulu clock string to minutes-past-midnight (0–1439).
fn hhmm_to_min(raw: &str) -> Option<i64> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let padded = if digits.len() == 3 {
        format!("0{digits}")
    } else {
        digits
    };
    if padded.len() != 4 {
        return None;
    }
    let hh: i64 = padded[0..2].parse().ok()?;
    let mm: i64 = padded[2..4].parse().ok()?;
    if hh >= 24 || mm >= 60 {
        return None;
    }
    Some(hh * 60 + mm)
}

/// Normalize a required HHMM field to canonical `HHMM`, or 400.
fn norm_hhmm(raw: &str) -> Result<String, ApiError> {
    let m = hhmm_to_min(raw).ok_or(ApiError::BadRequest)?;
    Ok(format!("{:02}{:02}", m / 60, m % 60))
}

/// Normalize a departure scope: uppercase ARTCC codes, single-spaced. Empty = all departures.
fn normalize_scope(raw: Option<&str>) -> String {
    raw.unwrap_or("")
        .split_whitespace()
        .map(|c| c.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve the program's HHMM window to concrete timestamps: start = the occurrence of
/// `start` nearest to `now` (±12h), end = the first occurrence of `end` strictly after start.
fn resolve_window(now: DateTime<Utc>, start: &str, end: &str) -> Option<(i64, i64)> {
    let sm = hhmm_to_min(start)?;
    let em = hhmm_to_min(end)?;
    let midnight = now.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
    let today_start = midnight + Duration::minutes(sm);
    // Pick the start occurrence (yesterday/today/tomorrow) closest to now.
    let start_ts = [-1i64, 0, 1]
        .into_iter()
        .map(|d| today_start + Duration::days(d))
        .min_by_key(|c| (*c - now).num_seconds().abs())?;
    let end_midnight = start_ts.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
    let mut end_ts = end_midnight + Duration::minutes(em);
    while end_ts <= start_ts {
        end_ts += Duration::days(1);
    }
    Some((start_ts.timestamp_millis(), end_ts.timestamp_millis()))
}

/// Resolve an AAR-step's HHMM to a concrete instant inside the resolved window (the first
/// occurrence at/after the window start). Steps outside the window are dropped.
fn resolve_step_ms(win_start_ms: i64, win_end_ms: i64, hhmm: &str) -> Option<i64> {
    let m = hhmm_to_min(hhmm)?;
    let ws = DateTime::from_timestamp_millis(win_start_ms)?;
    let base = ws.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
    let mut cand = base + Duration::minutes(m);
    while cand.timestamp_millis() < win_start_ms {
        cand += Duration::days(1);
    }
    (cand.timestamp_millis() <= win_end_ms).then_some(cand.timestamp_millis())
}

/// The program's (possibly time-varying) rate schedule over its resolved window.
fn rate_schedule(gdp: &GdpBody, win_start: i64, win_end: i64) -> gdp::RateSchedule {
    let steps: Vec<(i64, i32)> = gdp
        .aar_steps
        .0
        .iter()
        .filter_map(|s| resolve_step_ms(win_start, win_end, &s.start_time).map(|ms| (ms, s.aar)))
        .collect();
    gdp::RateSchedule::new(gdp.aar, win_start, &steps)
}

/// Validate AAR steps: each a valid HHMM + AAR in 1..=200 that lands inside the program
/// window (a step outside the window would be silently inert). Returns canonical HHMM steps.
fn validate_steps(
    steps: &[AarStep],
    win_start: i64,
    win_end: i64,
) -> Result<Vec<AarStep>, ApiError> {
    steps
        .iter()
        .map(|s| {
            if !(1..=200).contains(&s.aar) {
                return Err(ApiError::BadRequest);
            }
            let hhmm = norm_hhmm(&s.start_time)?;
            if resolve_step_ms(win_start, win_end, &hhmm).is_none() {
                return Err(ApiError::BadRequest); // outside the window → rejected
            }
            Ok(AarStep {
                start_time: hhmm,
                aar: s.aar,
            })
        })
        .collect()
}

/// Project the live feed into GDP inbounds for `icao` (raw classification + ETA + ETD +
/// departure ARTCC, no metering). Airborne/ground/proposed only — arrived flights dropped.
async fn live_inbounds(state: &AppState, icao: &str, now: DateTime<Utc>) -> Vec<gdp::Inbound> {
    let (snapshot, airports) = {
        let guard = state.feed.read().await;
        (guard.snapshot.clone(), guard.airports.clone())
    };
    let Some(snap) = snapshot else {
        return Vec::new();
    };
    let nav = state.nav.load_full();
    let winds = state.winds.load_full();
    let profiles = state.aircraft_profiles.load_full();
    let gates = state.gates.load_full();
    let runways = state.runways.clone();
    let taxi_estimate_samples = state.taxi_estimate_samples.load_full();
    let manual_exclusions =
        crate::handlers::flow::all_excluded_callsigns(state.flight_exclusions.load().as_ref());
    let icao = icao.to_owned();
    // `compute` resolves every arrival's filed route — pure CPU. Push it onto the blocking pool
    // (see `feed::flow_from_data`) rather than tying up an async worker.
    let Ok(flow) = tokio::task::spawn_blocking(move || {
        flow::compute(
            &icao,
            None, // no metering — we want the raw arrival picture
            &snap.data,
            airports.as_ref(),
            nav.as_ref(),
            winds.as_ref(),
            profiles.as_ref(),
            &HashMap::new(),
            gates.as_ref(),
            runways.as_ref(),
            taxi_estimate_samples.as_ref(),
            &manual_exclusions,
            now,
        )
    })
    .await
    else {
        return Vec::new();
    };
    // Resolve each origin field's owning ARTCC once, memoized across shared departures.
    let map = state.facilities.read().await;
    let mut artcc_of: HashMap<String, Option<String>> = HashMap::new();
    flow.flights
        .into_iter()
        .filter(|f| f.status != "arrived")
        .filter_map(|f| {
            let dep_artcc = artcc_of
                .entry(f.dep.clone())
                .or_insert_with(|| facilities::artcc_for_airport(&map, &f.dep.to_ascii_uppercase()))
                .clone();
            Some(gdp::Inbound {
                cs: f.callsign,
                dep: f.dep,
                dep_artcc,
                status: f.status,
                eta_ms: f.eta?.timestamp_millis(),
                etd_ms: f.etd.map(|e| e.timestamp_millis()),
            })
        })
        .collect()
}

fn ms(v: i64, fallback: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(v).unwrap_or(fallback)
}

/// Fresh RBS off the current feed for `gdp` — no frozen overrides applied.
async fn fresh_assignments(
    state: &AppState,
    gdp: &GdpBody,
    now: DateTime<Utc>,
) -> Result<(i64, i64, Vec<gdp::Assignment>), ApiError> {
    let (win_start, win_end) =
        resolve_window(now, &gdp.start_time, &gdp.end_time).ok_or(ApiError::Internal)?;
    let inbounds = live_inbounds(state, &gdp.airport.to_ascii_uppercase(), now).await;
    let scope: Vec<String> = gdp.scope.split_whitespace().map(String::from).collect();
    let rates = rate_schedule(gdp, win_start, win_end);
    let assignments = gdp::ration_by_schedule(
        inbounds,
        &rates,
        win_start,
        win_end,
        gdp.exempt_airborne,
        &scope,
        gdp.max_enroute_min,
    );
    Ok((win_start, win_end, assignments))
}

/// Freeze control times: run fresh RBS off the current feed with the program's current
/// params and persist the controlled slots (replacing any existing ones). Used on publish
/// and when revising a published program.
/// The frozen slot rows, the resolved program window, and the delay statistics for `gdp` — computed
/// entirely from the live feed, touching no database.
///
/// Split out of [`freeze_slots`] for #508. Publishing must write the slots and the generated advisory in
/// one transaction, and this is the expensive part: `fresh_assignments` runs the feed and RBS. Holding a
/// Postgres transaction open across it would contend with every other advisory create at the facility,
/// so the caller runs this *first* and opens the transaction afterwards with the results in hand.
async fn frozen_slots_for(
    state: &AppState,
    gdp: &GdpBody,
    now: DateTime<Utc>,
) -> Result<
    (
        DateTime<Utc>,
        DateTime<Utc>,
        Vec<gdp_repo::GdpSlotRow>,
        gdp::GdpStats,
    ),
    ApiError,
> {
    let (start_ms, end_ms, assignments) = fresh_assignments(state, gdp, now).await?;
    let slots: Vec<gdp_repo::GdpSlotRow> = assignments
        .iter()
        .filter(|a| a.controlled)
        .map(|a| gdp_repo::GdpSlotRow {
            callsign: a.cs.clone(),
            dep: a.dep.clone(),
            original_eta: ms(a.original_eta_ms, now),
            cta: ms(a.cta_ms, now),
            edct: a.edct_ms.map(|v| ms(v, now)),
            delay_min: a.delay_min as i32,
        })
        .collect();
    // The same assignments the slots came from, so the document's MAXIMUM/AVERAGE DELAY describe
    // exactly the rows that were frozen.
    let stats = gdp::program_stats(&assignments);
    let start = DateTime::from_timestamp_millis(start_ms).ok_or(ApiError::Internal)?;
    let end = DateTime::from_timestamp_millis(end_ms).ok_or(ApiError::Internal)?;
    Ok((start, end, slots, stats))
}

/// Run RBS off the current feed for `gdp`, applying frozen control times when published.
async fn assign_live(
    state: &AppState,
    pool: &sqlx::PgPool,
    gdp: &GdpBody,
    now: DateTime<Utc>,
) -> Result<(i64, i64, Vec<gdp::Assignment>), ApiError> {
    let (win_start, win_end, mut assignments) = fresh_assignments(state, gdp, now).await?;
    // Freeze: once published, matched flights hold their persisted control times.
    if gdp.status == "published" {
        let frozen = gdp_repo::list_slots(pool, &gdp.id).await?;
        let by_cs: HashMap<&str, &gdp_repo::GdpSlotRow> =
            frozen.iter().map(|s| (s.callsign.as_str(), s)).collect();
        for a in &mut assignments {
            if let Some(s) = by_cs.get(a.cs.as_str()) {
                a.cta_ms = s.cta.timestamp_millis();
                a.edct_ms = s.edct.map(|e| e.timestamp_millis());
                a.delay_min = s.delay_min as i64;
                a.controlled = true;
                a.frozen = true;
                a.exempt_reason = None;
            }
        }
    }
    Ok((win_start, win_end, assignments))
}

/// Assemble the board for `gdp` off the live feed.
async fn build_board(
    state: &AppState,
    pool: &sqlx::PgPool,
    gdp: &GdpBody,
) -> Result<GdpBoard, ApiError> {
    let now = Utc::now();
    let (win_start, win_end, assignments) = assign_live(state, pool, gdp, now).await?;
    let rates = rate_schedule(gdp, win_start, win_end);
    let demand = gdp::demand_bins(&assignments, win_start, win_end, &rates);
    let stats = gdp::program_stats(&assignments);

    let mut flights = Vec::new();
    let mut exempt = Vec::new();
    for a in assignments {
        let view = GdpFlightView {
            cs: a.cs,
            dep: a.dep,
            status: a.status,
            eta: ms(a.original_eta_ms, now),
            cta: ms(a.cta_ms, now),
            edct: a.edct_ms.map(|v| ms(v, now)),
            delay_min: a.delay_min,
            controlled: a.controlled,
            frozen: a.frozen,
            exempt_reason: a.exempt_reason,
        };
        if view.controlled {
            flights.push(view);
        } else {
            exempt.push(view);
        }
    }
    flights.sort_by_key(|f| f.cta);
    exempt.sort_by_key(|e| e.eta);

    Ok(GdpBoard {
        id: gdp.id.clone(),
        airport: gdp.airport.clone(),
        aar: gdp.aar,
        scope: gdp.scope.clone(),
        status: gdp.status.clone(),
        start_time: gdp.start_time.clone(),
        end_time: gdp.end_time.clone(),
        window_start: ms(win_start, now),
        window_end: ms(win_end, now),
        max_enroute_min: gdp.max_enroute_min,
        exempt_airborne: gdp.exempt_airborne,
        aar_steps: gdp.aar_steps.0.clone(),
        published: gdp.status == "published",
        flights,
        exempt,
        demand,
        stats,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/gdp",
    tag = "tmu",
    responses((status = 200, body = Vec<GdpBody>), (status = 401), (status = 503))
)]
pub async fn list_gdps(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpRead>,
) -> Result<Json<Vec<GdpBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut gdps = gdp_repo::list_gdps(pool).await?;
    restriction_artcc::stamp_gdps(&*state.facilities.read().await, &mut gdps);
    Ok(Json(gdps))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp",
    tag = "tmu",
    request_body = CreateGdpRequest,
    responses((status = 200, body = GdpBody), (status = 400), (status = 401), (status = 503))
)]
pub async fn create_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<CreateGdpRequest>,
) -> Result<Json<GdpBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let airport: String = payload
        .airport
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if airport.len() < 3 || airport.len() > 4 {
        return Err(ApiError::BadRequest);
    }
    if !(1..=200).contains(&payload.aar) {
        return Err(ApiError::BadRequest);
    }
    let start = norm_hhmm(&payload.start_time)?;
    let end = norm_hhmm(&payload.end_time)?;
    let scope = normalize_scope(payload.scope.as_deref());
    let max_enroute = payload.max_enroute_min.filter(|m| *m > 0);
    let (ws, we) = resolve_window(Utc::now(), &start, &end).ok_or(ApiError::BadRequest)?;
    let steps = validate_steps(&payload.aar_steps, ws, we)?;

    let id = gdp_repo::create_gdp(
        pool,
        &airport,
        payload.aar,
        &scope,
        &start,
        &end,
        max_enroute,
        payload.exempt_airborne,
        &steps,
        &user.id,
    )
    .await?;
    let mut gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    restriction_artcc::stamp_gdp(&*state.facilities.read().await, &mut gdp);
    Ok(Json(gdp))
}

/// Revise a GDP — change the AAR, window, tier, scope, or airborne policy. On a published
/// program this re-rations off the live feed and re-freezes control times (EDCTs may shift);
/// on a draft it just updates the parameters. Airport is immutable.
#[utoipa::path(
    put,
    path = "/api/v1/tmu/gdp/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    request_body = UpdateGdpRequest,
    responses((status = 200, body = GdpBoard), (status = 400), (status = 401), (status = 404), (status = 409), (status = 503))
)]
pub async fn revise_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateGdpRequest>,
) -> Result<Json<GdpBoard>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if !(1..=200).contains(&payload.aar) {
        return Err(ApiError::BadRequest);
    }
    let start = norm_hhmm(&payload.start_time)?;
    let end = norm_hhmm(&payload.end_time)?;
    let scope = normalize_scope(payload.scope.as_deref());
    let max_enroute = payload.max_enroute_min.filter(|m| *m > 0);
    let (ws, we) = resolve_window(Utc::now(), &start, &end).ok_or(ApiError::BadRequest)?;
    let steps = validate_steps(&payload.aar_steps, ws, we)?;

    // 404 if absent, 409 if terminal (expired/cancelled — nothing to revise).
    if gdp_repo::get_gdp(pool, &id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    if !gdp_repo::update_gdp(
        pool,
        &id,
        payload.aar,
        &scope,
        &start,
        &end,
        max_enroute,
        payload.exempt_airborne,
        &steps,
        &user.id,
    )
    .await?
    {
        return Err(ApiError::Conflict); // present but terminal
    }

    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // A live program re-rations + re-freezes with the new parameters, and its advisory no longer
    // describes it: the delay figures have moved. #461 settled that an advisory is cancelled and
    // reissued rather than rewritten, so that is what happens here (#508 AC4).
    //
    // The parameter update above is its own statement, deliberately: converting `update_gdp` to take a
    // transaction as well would be a wider change than this issue needs, and the failure mode is benign
    // -- if the freeze-and-reissue below fails, the program carries its new parameters with its previous
    // advisory still live, which is exactly today's behaviour. The cancel and the reissue are atomic
    // with each other, which is the part that matters: a program is never left with no live advisory.
    if gdp.status == "published" {
        let now = Utc::now();
        let (start, end, slots, stats) = frozen_slots_for(&state, &gdp, now).await?;
        let editorial = payload.advisory.unwrap_or_default();
        let iata = state.feed.read().await.iata.clone();
        let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
        gdp_repo::replace_slots_in(&mut tx, &id, &slots).await?;
        tmu_repo::cancel_program_advisory_tx(&mut tx, tmu_repo::AdvisoryProgram::Gdp(&id)).await?;
        generate_gdp_advisory(
            &mut tx,
            GdpDocInput {
                iata: &iata,
                gdp: &gdp,
                stats: &stats,
                window: (start, end),
                editorial: &editorial,
                now,
            },
            &user.id,
        )
        .await?;
        tx.commit().await.map_err(|_| ApiError::Internal)?;
    }
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/gdp/{id}/board",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 503))
)]
pub async fn get_gdp_board(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpRead>,
    Path(id): Path<String>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/publish",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    request_body(
        content = Option<PublishGdpRequest>,
        description = "Optional editorial fields for the advisory generated on publish (#508). Omit \
                       the body entirely and the advisory still generates, with those lines blank."
    ),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 409), (status = 503))
)]
pub async fn publish_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    editorial: Option<Json<PublishGdpRequest>>,
) -> Result<Json<GdpBoard>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let editorial = editorial.map(|Json(e)| e).unwrap_or_default();

    // Read the draft before publishing: the feed and RBS work below needs the program, and it must run
    // outside the transaction (see `frozen_slots_for`). The authoritative draft check is the UPDATE
    // inside the transaction — this one only avoids the expensive work for an obvious non-draft.
    let draft = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if draft.status != "draft" {
        return Err(ApiError::Conflict);
    }
    let now = Utc::now();
    let (start, end, slots, stats) = frozen_slots_for(&state, &draft, now).await?;
    // Read before the transaction opens, like every other feed read on this path: the document's
    // three-letter element is looked up in the IATA index (#508 review), and holding the feed lock
    // across the publish would serialise unrelated writes behind it.
    let iata = state.feed.read().await.iata.clone();

    // One transaction for every write (#508): the publish, the frozen control times, and the generated
    // advisory. An advisory must not exist for a program that did not publish, or the reverse.
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    if !gdp_repo::publish_gdp(&mut *tx, &id, &user.id).await? {
        return Err(ApiError::Conflict); // not a draft (or absent), or lost a race
    }
    let gdp = gdp_repo::get_gdp(&mut *tx, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    gdp_repo::replace_slots_in(&mut tx, &id, &slots).await?;
    generate_gdp_advisory(
        &mut tx,
        GdpDocInput {
            iata: &iata,
            gdp: &gdp,
            stats: &stats,
            window: (start, end),
            editorial: &editorial,
            now,
        },
        &user.id,
    )
    .await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    Ok(Json(build_board(&state, pool, &gdp).await?))
}

/// Create the GDP's advisory inside the publish transaction (#508, carrying #461's AC3).
///
/// The document is derived from the program rather than retyped, because a GDP advisory and its
/// `tmu.gdp` row are the same event. `structured` carries the fields and the stored `body` is their
/// rendering — `repos::tmu::create_advisory_tx` renders it, since the advisory *number* is allocated in
/// this transaction and the document carries it.
///
/// Issued as `DCC`: these are vATCSCC documents, which both reference fixtures confirm. The facility
/// drives the advisory-number sequence; the airport and ARTCC appear in the header's element slot.
/// Everything the document is derived from, grouped rather than passed positionally.
///
/// Adding the feed's IATA index took the helper to eight arguments, which `clippy::too_many_arguments`
/// rejects. Grouping beats an `allow`: the publish and revise paths both build one of these, so they
/// cannot drift into disagreeing about the order of six same-ish references.
struct GdpDocInput<'a> {
    iata: &'a crate::feed::airports::IataMap,
    gdp: &'a GdpBody,
    stats: &'a gdp::GdpStats,
    window: (DateTime<Utc>, DateTime<Utc>),
    editorial: &'a PublishGdpRequest,
    now: DateTime<Utc>,
}

async fn generate_gdp_advisory(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: GdpDocInput<'_>,
    author: &str,
) -> Result<String, ApiError> {
    let GdpDocInput {
        iata,
        gdp,
        stats,
        window,
        editorial,
        now,
    } = input;
    let doc = crate::advisory::gdp_advisory_from(iata, gdp, stats, window, editorial, now);
    let req = CreateAdvisoryRequest {
        facility: "DCC".to_string(),
        kind: crate::models::ADVISORY_KIND_GDP.to_string(),
        // Empty: the body is rendered from `structured` during creation, never supplied here.
        body: String::new(),
        structured: Some(serde_json::to_value(&doc).map_err(|_| ApiError::Internal)?),
        decoded: None,
    };
    tmu_repo::create_advisory_tx(
        tx,
        &req,
        author,
        Some(tmu_repo::AdvisoryProgram::Gdp(&gdp.id)),
    )
    .await
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/cancel",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 200, body = GdpBody), (status = 401), (status = 409), (status = 503))
)]
pub async fn cancel_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path(id): Path<String>,
) -> Result<Json<GdpBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !gdp_repo::cancel_gdp(pool, &id).await? {
        return Err(ApiError::Conflict);
    }
    let mut gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    restriction_artcc::stamp_gdp(&*state.facilities.read().await, &mut gdp);
    Ok(Json(gdp))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/gdp/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 204), (status = 401), (status = 404), (status = 503))
)]
pub async fn delete_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !gdp_repo::delete_gdp(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Lock a controlled flight's current (advisory) control time into a frozen slot. Used to
/// pin a pop-up that appeared after publish so its EDCT stops drifting.
#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/slots/{callsign}",
    tag = "tmu",
    params(
        ("id" = String, Path, description = "GDP id"),
        ("callsign" = String, Path, description = "Flight callsign")
    ),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 409), (status = 503))
)]
pub async fn lock_slot(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if gdp.status != "published" {
        return Err(ApiError::Conflict); // only a live program has slots to lock
    }
    let now = Utc::now();
    let callsign = callsign.to_ascii_uppercase();
    // Freeze the flight's current advisory assignment (fresh RBS, no existing overrides).
    let (_s, _e, assignments) = fresh_assignments(&state, &gdp, now).await?;
    let a = assignments
        .iter()
        .find(|a| a.controlled && a.cs.eq_ignore_ascii_case(&callsign))
        .ok_or(ApiError::NotFound)?; // not an eligible controlled flight
    gdp_repo::upsert_slot(
        pool,
        &gdp.id,
        &gdp_repo::GdpSlotRow {
            callsign: a.cs.clone(),
            dep: a.dep.clone(),
            original_eta: ms(a.original_eta_ms, now),
            cta: ms(a.cta_ms, now),
            edct: a.edct_ms.map(|v| ms(v, now)),
            delay_min: a.delay_min as i32,
        },
    )
    .await?;
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

/// Unlock (remove) a frozen slot — the flight reverts to a live advisory control time.
#[utoipa::path(
    delete,
    path = "/api/v1/tmu/gdp/{id}/slots/{callsign}",
    tag = "tmu",
    params(
        ("id" = String, Path, description = "GDP id"),
        ("callsign" = String, Path, description = "Flight callsign")
    ),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 503))
)]
pub async fn unlock_slot(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !gdp_repo::delete_slot(pool, &gdp.id, &callsign.to_ascii_uppercase()).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

/// Compress the program: reclaim capacity freed by departed/cancelled flights by pulling each
/// frozen slot to its current fresh-RBS time — earlier only, never later than already issued.
/// Frozen flights that have left the arrival picture are dropped.
#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/compress",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 409), (status = 503))
)]
pub async fn compress_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path(id): Path<String>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if gdp.status != "published" {
        return Err(ApiError::Conflict);
    }
    let now = Utc::now();
    let (_s, _e, fresh) = fresh_assignments(&state, &gdp, now).await?;
    let frozen = gdp_repo::list_slots(pool, &gdp.id).await?;
    let frozen_cta: HashMap<&str, i64> = frozen
        .iter()
        .map(|s| (s.callsign.as_str(), s.cta.timestamp_millis()))
        .collect();

    // Keep only flights that are both currently controllable and previously frozen; pull each
    // to its compressed time. Frozen flights no longer inbound simply fall away.
    let mut new_slots = Vec::new();
    for a in &fresh {
        if !a.controlled {
            continue;
        }
        let Some(&frozen_cta_ms) = frozen_cta.get(a.cs.as_str()) else {
            continue;
        };
        let (cta_ms, edct_ms, delay_min) = gdp::compress_slot(a, frozen_cta_ms);
        new_slots.push(gdp_repo::GdpSlotRow {
            callsign: a.cs.clone(),
            dep: a.dep.clone(),
            original_eta: ms(a.original_eta_ms, now),
            cta: ms(cta_ms, now),
            edct: edct_ms.map(|v| ms(v, now)),
            delay_min: delay_min as i32,
        });
    }
    gdp_repo::replace_slots(pool, &gdp.id, &new_slots).await?;
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope_test_support::{grant, seed_user, send, session_cookie, test_state};
    use sqlx::PgPool;
    use std::collections::HashMap;

    /// A draft GDP with distinctive derived values, so a test can tell a real derivation from a
    /// hard-coded one: `KJFK` must shorten to `JFK`, and the stepped rates must appear in order.
    async fn draft_gdp(pool: &PgPool, author: &str) -> String {
        gdp_repo::create_gdp(
            pool,
            "KJFK",
            40,
            "ZBW ZDC",
            "1415",
            "2315",
            None,
            false,
            &[
                AarStep {
                    start_time: "1415".into(),
                    aar: 40,
                },
                AarStep {
                    start_time: "1615".into(),
                    aar: 30,
                },
                AarStep {
                    start_time: "1815".into(),
                    aar: 25,
                },
            ],
            author,
        )
        .await
        .unwrap()
    }

    async fn advisories_for_gdp(pool: &PgPool, gdp_id: &str) -> Vec<(String, String, String)> {
        sqlx::query_as::<_, (String, String, String)>(
            "select id, status, body from tmu.advisories where gdp_id = $1 order by number",
        )
        .bind(gdp_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// AC1 — publishing a GDP produces its advisory, in the same transaction as the publish.
    #[sqlx::test]
    async fn publishing_a_gdp_generates_its_advisory(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.gdp.publish", None).await;
        let id = draft_gdp(&pool, &user).await;

        let status = send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/gdp/{id}/publish"),
            &cookie,
            None,
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);

        let advisories = advisories_for_gdp(&pool, &id).await;
        assert_eq!(
            advisories.len(),
            1,
            "exactly one advisory, linked to the program"
        );
        assert!(
            !advisories[0].2.trim().is_empty(),
            "the document should have been rendered, not left empty"
        );
    }

    /// AC1, the other direction — a publish that conflicts must leave no advisory behind. The whole
    /// point of the single transaction.
    #[sqlx::test]
    async fn a_conflicting_publish_generates_no_advisory(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.gdp.publish", None).await;
        let id = draft_gdp(&pool, &user).await;
        let uri = format!("/api/v1/tmu/gdp/{id}/publish");

        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::OK
        );
        // Second publish: already published, so nothing may be written.
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::CONFLICT
        );

        assert_eq!(
            advisories_for_gdp(&pool, &id).await.len(),
            1,
            "the refused publish must not have issued a second advisory"
        );
    }

    /// AC1's real guarantee, which the happy-path tests cannot show: if anything in the transaction
    /// fails, the publish itself is rolled back. Two sequential transactions would look identical on a
    /// successful publish — the difference only appears when a later write fails, so that is what this
    /// induces, by linking the advisory to a program id that violates the foreign key.
    ///
    /// Drives the repo layer directly rather than the router: the point is the transaction boundary, and
    /// there is no way to make the handler's own advisory insert fail from outside.
    #[sqlx::test]
    async fn a_failed_advisory_write_rolls_the_publish_back(pool: PgPool) {
        let user = seed_user(&pool).await;
        let id = draft_gdp(&pool, &user).await;

        let mut tx = pool.begin().await.unwrap();
        assert!(gdp_repo::publish_gdp(&mut *tx, &id, &user).await.unwrap());
        // A *valid* document, derived the way the handler derives it. A stub payload would make
        // `advisory_body` reject it before the foreign key was ever reached, and the test would then
        // pass while proving something else entirely — which it did, until mutation testing showed it.
        let gdp = gdp_repo::get_gdp(&mut *tx, &id).await.unwrap().unwrap();
        let doc = crate::advisory::gdp_advisory_from(
            &Default::default(),
            &gdp,
            &gdp::program_stats(&[]),
            (Utc::now(), Utc::now()),
            &PublishGdpRequest::default(),
            Utc::now(),
        );
        let req = CreateAdvisoryRequest {
            facility: "DCC".into(),
            kind: crate::models::ADVISORY_KIND_GDP.into(),
            body: String::new(),
            structured: Some(serde_json::to_value(&doc).unwrap()),
            decoded: None,
        };
        let failed = tmu_repo::create_advisory_tx(
            &mut tx,
            &req,
            &user,
            Some(tmu_repo::AdvisoryProgram::Gdp("no-such-program")),
        )
        .await;
        assert!(
            failed.is_err(),
            "the bogus program link should violate the FK"
        );
        drop(tx); // the transaction is abandoned, exactly as the handler's `?` would

        let after = gdp_repo::get_gdp(&pool, &id).await.unwrap().unwrap();
        assert_eq!(
            after.status, "draft",
            "the publish must have rolled back with the failed advisory write"
        );
        assert!(
            advisories_for_gdp(&pool, &id).await.is_empty(),
            "and no advisory may survive"
        );
    }

    /// AC3 — **the anti-drift test.** The document must carry the program's own values, so that
    /// changing the program changes the document. Mutating any of these derivations to a constant turns
    /// this red.
    #[sqlx::test]
    async fn the_advisory_carries_the_programs_values(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.gdp.publish", None).await;
        let id = draft_gdp(&pool, &user).await;

        send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/gdp/{id}/publish"),
            &cookie,
            None,
        )
        .await;

        let body = advisories_for_gdp(&pool, &id).await.remove(0).2;
        assert!(
            body.contains("CTL ELEMENT: JFK"),
            "the ICAO KJFK should print as the document's three-letter form; got:\n{body}"
        );
        assert!(
            body.contains("PROGRAM RATE: 40/30/25"),
            "the stepped rates should come from the program's aar_steps, in order; got:\n{body}"
        );
        assert!(
            body.contains("CDM GROUND DELAY PROGRAM"),
            "the GDP header line; got:\n{body}"
        );
        assert!(
            body.contains("1415Z") && body.contains("2315Z"),
            "the program window should appear; got:\n{body}"
        );
    }

    /// The element lookup has to survive the trip from the feed into the document, and nothing else
    /// proves it does.
    ///
    /// The derivation's own unit tests pass a map directly, so discarding the one the handler looked up
    /// — `gdp_advisory_from(&Default::default(), ..)` — left every test green: an empty index falls back
    /// to the `K`-strip, and the other GDP tests all publish at `KJFK`, where the fallback is correct.
    /// Publishing a *non-`K`* airport is what tells the two apart. `PHNL` renders `HNL` only if the real
    /// index arrived; with an empty one it renders `PHNL`.
    #[sqlx::test]
    async fn the_feeds_iata_index_reaches_the_document(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        state.feed.write().await.iata = std::sync::Arc::new(
            [("HNL".to_string(), "PHNL".to_string())]
                .into_iter()
                .collect(),
        );
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.gdp.publish", None).await;
        let id = gdp_repo::create_gdp(
            &pool,
            "PHNL",
            30,
            "ZAK",
            "1415",
            "2315",
            None,
            false,
            &[],
            &user,
        )
        .await
        .unwrap();

        send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/gdp/{id}/publish"),
            &cookie,
            None,
        )
        .await;

        let body = advisories_for_gdp(&pool, &id).await.remove(0).2;
        assert!(
            body.contains("CTL ELEMENT: HNL"),
            "Honolulu's three-letter form must come from the feed's index, not a K-strip; got:\n{body}"
        );
        assert!(
            !body.contains("PHNL"),
            "the raw ICAO must not appear in the document; got:\n{body}"
        );
    }

    /// AC2 — the editorial fields the data cannot supply arrive on the publish request and reach the
    /// document.
    #[sqlx::test]
    async fn editorial_fields_sent_at_publish_reach_the_document(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.gdp.publish", None).await;
        let id = draft_gdp(&pool, &user).await;

        let status = send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/gdp/{id}/publish"),
            &cookie,
            Some(serde_json::json!({
                "delay_limit": "240",
                "impacting_condition": "WEATHER / THUNDERSTORMS",
                "pop_up_factor": "MEDIUM",
                "delay_assignment_mode": "GAAP",
            })),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);

        let body = advisories_for_gdp(&pool, &id).await.remove(0).2;
        assert!(body.contains("DELAY LIMIT: 240"), "got:\n{body}");
        assert!(
            body.contains("IMPACTING CONDITION: WEATHER / THUNDERSTORMS"),
            "got:\n{body}"
        );
        assert!(body.contains("POP-UP FACTOR: MEDIUM"), "got:\n{body}");
        assert!(
            body.contains("DELAY ASSIGNMENT MODE: GAAP"),
            "the supplied mode should override the DAS default; got:\n{body}"
        );
    }

    /// A GDP must stay publishable in a hurry, so the editorial body is optional and its absence falls
    /// back to the documented default rather than failing.
    #[sqlx::test]
    async fn publishing_without_an_editorial_body_still_generates(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.gdp.publish", None).await;
        let id = draft_gdp(&pool, &user).await;

        send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/gdp/{id}/publish"),
            &cookie,
            None,
        )
        .await;

        let body = advisories_for_gdp(&pool, &id).await.remove(0).2;
        assert!(
            body.contains("DELAY ASSIGNMENT MODE: DAS"),
            "the default mode; got:\n{body}"
        );
        assert!(
            body.contains("DELAY LIMIT:"),
            "an absent editorial field still prints its bare label; got:\n{body}"
        );
    }

    /// AC4 — #461 settled that an advisory is cancelled and reissued, never rewritten. Revising a
    /// published program must therefore retire its advisory and issue a fresh one.
    #[sqlx::test]
    async fn revising_a_published_gdp_reissues_its_advisory(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.gdp.publish", None).await;
        grant(&pool, &user, "tmu.gdp.create", None).await;
        let id = draft_gdp(&pool, &user).await;
        send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/gdp/{id}/publish"),
            &cookie,
            None,
        )
        .await;
        let first = advisories_for_gdp(&pool, &id).await;
        assert_eq!(first.len(), 1);

        let status = send(
            &state,
            http::Method::PUT,
            &format!("/api/v1/tmu/gdp/{id}"),
            &cookie,
            Some(serde_json::json!({
                "aar": 20,
                "scope": "ZBW ZDC",
                "start_time": "1415",
                "end_time": "2315",
                "exempt_airborne": false,
                "aar_steps": [],
            })),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);

        let after = advisories_for_gdp(&pool, &id).await;
        assert_eq!(
            after.len(),
            2,
            "the old advisory is kept, cancelled, and a new one issued"
        );
        let old = after
            .iter()
            .find(|a| a.0 == first[0].0)
            .expect("the original");
        assert_eq!(
            old.1, "cancelled",
            "the previous advisory is cancelled, not rewritten"
        );
        assert_eq!(
            old.2, first[0].2,
            "and its document is left exactly as issued"
        );
        let live: Vec<&(String, String, String)> =
            after.iter().filter(|a| a.1 != "cancelled").collect();
        assert_eq!(live.len(), 1, "exactly one live advisory remains");
        assert!(
            live[0].2.contains("PROGRAM RATE: 20"),
            "the reissued document carries the revised rate; got:\n{}",
            live[0].2
        );
    }
}
