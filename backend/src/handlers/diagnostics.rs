//! Desktop diagnostics reports (#629): the desktop app's upload, and the staff-only admin view.
//!
//! The upload is OIS's first client-composed content, so it is bounded on every axis: only a signed-in
//! *desktop* session may send one (the button exists only there, and the desktop's Rust is the only
//! caller — which is why it is not in the OpenAPI document), at most [`MAX_REPORTS_PER_HOUR`] per user,
//! at most [`MAX_UPLOAD_BYTES`] per request (the route's `DefaultBodyLimit`; `Multipart` has no implicit
//! cap), and the logs must be gzip. Who sent it comes from the session, never from the bundle.

use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Multipart, Path, Query, State, multipart::MultipartError},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::{
        context::{CurrentUser, SessionToken},
        permissions::{DiagnosticsReportsDelete, DiagnosticsReportsRead},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{DiagnosticsReport, DiagnosticsReportPage},
    repos::diagnostics::{self as repo, NewReport},
    state::AppState,
};

/// The largest upload accepted, applied to the route as its `DefaultBodyLimit`.
pub const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;

/// How many reports one user may send in an hour.
pub const MAX_REPORTS_PER_HOUR: i64 = 5;

/// The longest note kept, in characters — the same order as other free text OIS accepts.
const MAX_NOTE_CHARS: usize = 4_000;

/// Only the desktop app's session token carries this prefix (`auth::middleware`).
const DESKTOP_SESSION_PREFIX: &str = "ois_dsk_";

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// `POST /api/v1/diagnostics/reports` — multipart `meta` (JSON) + `logs` (gzip). Returns `201 {id}`.
pub async fn upload_report(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(session): Extension<SessionToken>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let user = current_user.ok_or(ApiError::Unauthorized)?;
    if !session
        .0
        .as_deref()
        .is_some_and(|token| token.starts_with(DESKTOP_SESSION_PREFIX))
    {
        return Err(ApiError::Unauthorized);
    }
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if repo::count_since(pool, &user.id, Utc::now() - Duration::hours(1)).await?
        >= MAX_REPORTS_PER_HOUR
    {
        return Err(ApiError::TooManyRequests);
    }

    let (mut meta, mut logs): (Option<Bytes>, Option<Bytes>) = (None, None);
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("meta") => meta = Some(field.bytes().await.map_err(multipart_error)?),
            Some("logs") => logs = Some(field.bytes().await.map_err(multipart_error)?),
            _ => {}
        }
    }
    let meta: Value = meta
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(Value::is_object)
        .ok_or(ApiError::BadRequest)?;
    let logs = logs
        .filter(|bytes| bytes.starts_with(&GZIP_MAGIC))
        .ok_or(ApiError::BadRequest)?;
    let note = text(&meta, &["note"]);
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(ApiError::BadRequest);
    }

    let id = repo::insert_report(
        pool,
        &NewReport {
            user_id: &user.id,
            app_version: text(&meta, &["app_version"]),
            os: text(&meta, &["os"]),
            os_version: text(&meta, &["os_version"]),
            arch: text(&meta, &["arch"]),
            webview_version: text(&meta, &["webview_version"]),
            window_label: text(&meta, &["context", "window_label"]),
            route: text(&meta, &["context", "route"]),
            note,
            meta: &meta,
            logs: &logs,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// A body over the limit surfaces as a multipart read error; say so rather than "bad request".
fn multipart_error(error: MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge
    } else {
        ApiError::BadRequest
    }
}

/// The string at `path` in `meta`, or `""`.
fn text<'a>(meta: &'a Value, path: &[&str]) -> &'a str {
    path.iter()
        .try_fold(meta, |value, key| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

#[derive(Deserialize)]
pub struct ReportListQuery {
    page: Option<i64>,
    page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/diagnostics",
    tag = "diagnostics",
    params(
        ("page" = Option<i64>, Query, description = "1-based page (default 1)"),
        ("page_size" = Option<i64>, Query, description = "Page size (default 50, max 100)")
    ),
    responses((status = 200, body = DiagnosticsReportPage), (status = 401)),
    security(("session" = ["diagnostics.reports.read"]), ("api_key" = ["diagnostics.reports.read"]), ("service_account" = ["diagnostics.reports.read"]))
)]
pub async fn list_reports(
    State(state): State<AppState>,
    _permission: RequirePermission<DiagnosticsReportsRead>,
    Query(query): Query<ReportListQuery>,
) -> Result<Json<DiagnosticsReportPage>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 100);
    Ok(Json(DiagnosticsReportPage {
        total: repo::count_reports(pool).await?,
        items: repo::list_reports(pool, page_size, (page - 1) * page_size).await?,
        page,
        page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/diagnostics/{id}",
    tag = "diagnostics",
    params(("id" = String, Path, description = "Report id")),
    responses((status = 200, body = DiagnosticsReport), (status = 401), (status = 404)),
    security(("session" = ["diagnostics.reports.read"]), ("api_key" = ["diagnostics.reports.read"]), ("service_account" = ["diagnostics.reports.read"]))
)]
pub async fn get_report(
    State(state): State<AppState>,
    _permission: RequirePermission<DiagnosticsReportsRead>,
    Path(id): Path<String>,
) -> Result<Json<DiagnosticsReport>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    repo::get_report(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// The report's logs, as the gzip file the desktop sent.
#[utoipa::path(
    get,
    path = "/api/v1/admin/diagnostics/{id}/logs",
    tag = "diagnostics",
    params(("id" = String, Path, description = "Report id")),
    responses(
        (status = 200, description = "The gzipped log files", content_type = "application/gzip", body = Vec<u8>),
        (status = 401),
        (status = 404)
    ),
    security(("session" = ["diagnostics.reports.read"]), ("api_key" = ["diagnostics.reports.read"]), ("service_account" = ["diagnostics.reports.read"]))
)]
pub async fn get_report_logs(
    State(state): State<AppState>,
    _permission: RequirePermission<DiagnosticsReportsRead>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let logs = repo::get_logs(pool, &id).await?.ok_or(ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"ois-diagnostics-{id}.log.gz\""),
            ),
        ],
        logs,
    )
        .into_response())
}

/// Deletes a report — e.g. when its sender asks for their data to be erased.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/diagnostics/{id}",
    tag = "diagnostics",
    params(("id" = String, Path, description = "Report id")),
    responses((status = 204), (status = 401), (status = 404)),
    security(("session" = ["diagnostics.reports.delete"]), ("api_key" = ["diagnostics.reports.delete"]), ("service_account" = ["diagnostics.reports.delete"]))
)]
pub async fn delete_report(
    State(state): State<AppState>,
    _permission: RequirePermission<DiagnosticsReportsDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if repo::delete_report(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use sqlx::PgPool;
    use tower::ServiceExt;

    use super::*;
    use crate::scope_test_support::{grant, seed_user, session_cookie, test_state};

    const BOUNDARY: &str = "oisdiagnosticsboundary";
    /// The two-byte gzip header plus a little padding — the handler checks only the magic.
    const GZIP: &[u8] = &[0x1f, 0x8b, 0x08, 0x00, 0x00];

    /// A desktop session for `user_id`, as the bearer the desktop app sends.
    async fn desktop_bearer(pool: &PgPool, user_id: &str) -> String {
        // The session lookup decodes a CID; `seed_user` rows have none (as `session_cookie` notes).
        sqlx::query(
            "update identity.users \
             set cid = coalesce(cid, (select coalesce(max(cid), 0) + 1 from identity.users)) \
             where id = $1",
        )
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
        let token = format!("ois_dsk_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(
            "insert into identity.sessions (session_token, user_id, expires_at, kind) \
             values ($1, $2, now() + interval '1 hour', 'desktop')",
        )
        .bind(&token)
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
        format!("Bearer {token}")
    }

    fn multipart(meta: &Value, logs: &[u8]) -> Vec<u8> {
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"meta\"\r\n\
             Content-Type: application/json\r\n\r\n{meta}\r\n\
             --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"logs\"; filename=\"logs.gz\"\r\n\
             Content-Type: application/gzip\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(logs);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        body
    }

    fn meta() -> Value {
        json!({
            "app_version": "0.2.0", "os": "macos", "os_version": "15.1", "arch": "aarch64",
            "webview_version": "620.1", "note": "the map went white",
            "context": { "window_label": "main", "route": "/flow", "webgl2": true }
        })
    }

    struct Reply {
        status: StatusCode,
        body: Vec<u8>,
    }

    async fn call(
        state: &AppState,
        method: &str,
        uri: &str,
        auth: (&str, &str),
        body: Option<Vec<u8>>,
    ) -> Reply {
        let mut request = http::Request::builder().method(method).uri(uri);
        if !auth.1.is_empty() {
            request = request.header(auth.0, auth.1);
        }
        let request = match body {
            Some(body) => request
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .body(Body::from(body)),
            None => request.body(Body::empty()),
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
        Reply { status, body }
    }

    async fn upload(state: &AppState, bearer: &str, body: Vec<u8>) -> Reply {
        call(
            state,
            "POST",
            "/api/v1/diagnostics/reports",
            ("authorization", bearer),
            Some(body),
        )
        .await
    }

    async fn report_count(pool: &PgPool) -> i64 {
        sqlx::query_scalar("select count(*) from diagnostics.reports")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// AC3: a desktop session's report is stored against the session's user, with the platform and
    /// context fields picked out of the bundle.
    #[sqlx::test]
    async fn a_desktop_session_uploads_a_report(pool: PgPool) {
        let user = seed_user(&pool).await;
        let bearer = desktop_bearer(&pool, &user).await;
        let state = test_state(pool.clone(), Default::default());

        let reply = upload(&state, &bearer, multipart(&meta(), GZIP)).await;

        assert_eq!(reply.status, StatusCode::CREATED);
        let id = serde_json::from_slice::<Value>(&reply.body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (owner, os_version, route, note, logs): (String, String, String, String, Vec<u8>) =
            sqlx::query_as(
                "select user_id, os_version, route, note, logs from diagnostics.reports where id = $1",
            )
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(owner, user, "who sent it comes from the session");
        assert_eq!((os_version.as_str(), route.as_str()), ("15.1", "/flow"));
        assert_eq!(note, "the map went white");
        assert_eq!(logs, GZIP);
    }

    /// The button exists only on the desktop; a web session is not a desktop and is refused.
    #[sqlx::test]
    async fn a_web_session_or_no_session_cannot_upload(pool: PgPool) {
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        let state = test_state(pool.clone(), Default::default());

        let by_cookie = call(
            &state,
            "POST",
            "/api/v1/diagnostics/reports",
            ("cookie", &cookie),
            Some(multipart(&meta(), GZIP)),
        )
        .await;
        assert_eq!(by_cookie.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            upload(&state, "", multipart(&meta(), GZIP)).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(report_count(&pool).await, 0);
    }

    /// AC4: `Multipart` inherits no limit, so the route's own cap is what stops an oversized bundle —
    /// with a clear 413, and nothing stored.
    #[sqlx::test]
    async fn an_upload_over_the_cap_is_refused_with_413(pool: PgPool) {
        let user = seed_user(&pool).await;
        let bearer = desktop_bearer(&pool, &user).await;
        let state = test_state(pool.clone(), Default::default());
        let mut logs = GZIP.to_vec();
        logs.resize(MAX_UPLOAD_BYTES + 1, 0);

        let reply = upload(&state, &bearer, multipart(&meta(), &logs)).await;

        assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(&reply.body[..], br#"{"error":"payload_too_large"}"#);
        assert_eq!(report_count(&pool).await, 0);

        // The far side: just under the cap is accepted.
        let mut logs = GZIP.to_vec();
        logs.resize(MAX_UPLOAD_BYTES - 64 * 1024, 0);
        assert_eq!(
            upload(&state, &bearer, multipart(&meta(), &logs))
                .await
                .status,
            StatusCode::CREATED
        );
    }

    #[sqlx::test]
    async fn a_sixth_report_within_the_hour_is_refused(pool: PgPool) {
        let user = seed_user(&pool).await;
        let bearer = desktop_bearer(&pool, &user).await;
        let state = test_state(pool.clone(), Default::default());

        for _ in 0..MAX_REPORTS_PER_HOUR {
            assert_eq!(
                upload(&state, &bearer, multipart(&meta(), GZIP))
                    .await
                    .status,
                StatusCode::CREATED
            );
        }
        let refused = upload(&state, &bearer, multipart(&meta(), GZIP)).await;
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);

        // An hour on, the oldest no longer count.
        sqlx::query("update diagnostics.reports set created_at = now() - interval '61 minutes'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            upload(&state, &bearer, multipart(&meta(), GZIP))
                .await
                .status,
            StatusCode::CREATED
        );
    }

    #[sqlx::test]
    async fn logs_that_are_not_gzip_or_a_missing_meta_are_bad_requests(pool: PgPool) {
        let user = seed_user(&pool).await;
        let bearer = desktop_bearer(&pool, &user).await;
        let state = test_state(pool.clone(), Default::default());

        let plain = upload(&state, &bearer, multipart(&meta(), b"plain text logs")).await;
        assert_eq!(plain.status, StatusCode::BAD_REQUEST);
        let not_object = upload(&state, &bearer, multipart(&json!("a string"), GZIP)).await;
        assert_eq!(not_object.status, StatusCode::BAD_REQUEST);
        let long_note = json!({ "note": "x".repeat(MAX_NOTE_CHARS + 1) });
        assert_eq!(
            upload(&state, &bearer, multipart(&long_note, GZIP))
                .await
                .status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(report_count(&pool).await, 0);
    }

    /// AC7 + AC9: reading needs `diagnostics.reports.read`, deleting needs `.delete` on top.
    #[sqlx::test]
    async fn reports_are_read_and_deleted_only_with_their_permissions(pool: PgPool) {
        let sender = seed_user(&pool).await;
        let bearer = desktop_bearer(&pool, &sender).await;
        let state = test_state(pool.clone(), Default::default());
        let created = upload(&state, &bearer, multipart(&meta(), GZIP)).await;
        let id = serde_json::from_slice::<Value>(&created.body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (list, one, logs) = (
            "/api/v1/admin/diagnostics".to_string(),
            format!("/api/v1/admin/diagnostics/{id}"),
            format!("/api/v1/admin/diagnostics/{id}/logs"),
        );

        let stranger = seed_user(&pool).await;
        let stranger = session_cookie(&pool, &stranger).await;
        for uri in [&list, &one, &logs] {
            let reply = call(&state, "GET", uri, ("cookie", &stranger), None).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{uri}");
        }

        let reader = seed_user(&pool).await;
        grant(&pool, &reader, "diagnostics.reports.read", None).await;
        let reader = session_cookie(&pool, &reader).await;
        let page: Value = serde_json::from_slice(
            &call(&state, "GET", &list, ("cookie", &reader), None)
                .await
                .body,
        )
        .unwrap();
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["id"], id.as_str());
        let report = call(&state, "GET", &one, ("cookie", &reader), None).await;
        assert_eq!(report.status, StatusCode::OK);
        let report: Value = serde_json::from_slice(&report.body).unwrap();
        assert_eq!(report["note"], "the map went white");
        assert_eq!(report["meta"]["context"]["webgl2"], true);
        assert_eq!(
            call(&state, "GET", &logs, ("cookie", &reader), None)
                .await
                .body,
            GZIP
        );
        assert_eq!(
            call(&state, "DELETE", &one, ("cookie", &reader), None)
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "reading is not deleting"
        );

        let eraser = seed_user(&pool).await;
        grant(&pool, &eraser, "diagnostics.reports.delete", None).await;
        let eraser = session_cookie(&pool, &eraser).await;
        assert_eq!(
            call(&state, "DELETE", &one, ("cookie", &eraser), None)
                .await
                .status,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(&state, "DELETE", &one, ("cookie", &eraser), None)
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(report_count(&pool).await, 0);
    }

    /// AC9: erasing the person erases their reports.
    #[sqlx::test]
    async fn deleting_the_user_deletes_their_reports(pool: PgPool) {
        let user = seed_user(&pool).await;
        let bearer = desktop_bearer(&pool, &user).await;
        let state = test_state(pool.clone(), Default::default());
        upload(&state, &bearer, multipart(&meta(), GZIP)).await;
        assert_eq!(report_count(&pool).await, 1);

        sqlx::query("delete from identity.users where id = $1")
            .bind(&user)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(report_count(&pool).await, 0);
    }

    /// AC8: the prune takes reports past 30 days and leaves those inside it. Absolute ages either side
    /// of the window, not ages derived from the constant.
    #[sqlx::test]
    async fn the_prune_removes_only_reports_past_thirty_days(pool: PgPool) {
        let user = seed_user(&pool).await;
        for days in [29, 31] {
            sqlx::query(
                "insert into diagnostics.reports (user_id, created_at, meta, logs, logs_bytes) \
                 values ($1, now() - make_interval(days => $2), '{}', '\\x1f8b', 2)",
            )
            .bind(&user)
            .bind(days)
            .execute(&pool)
            .await
            .unwrap();
        }

        let removed = crate::jobs::diagnostics_report_prune_once(&pool, Utc::now())
            .await
            .unwrap();

        assert_eq!(removed, 1);
        let left: i32 = sqlx::query_scalar(
            "select extract(day from now() - created_at)::int from diagnostics.reports",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(left, 29);
    }
}
