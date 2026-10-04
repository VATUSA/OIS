//! Desktop diagnostics reports (#629).

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{DiagnosticsReport, DiagnosticsReportSummary},
};

/// A report as received, ready to store. `logs` is the gzip body exactly as sent.
pub struct NewReport<'a> {
    pub user_id: &'a str,
    pub app_version: &'a str,
    pub os: &'a str,
    pub os_version: &'a str,
    pub arch: &'a str,
    pub webview_version: &'a str,
    pub window_label: &'a str,
    pub route: &'a str,
    pub note: &'a str,
    pub meta: &'a Value,
    pub logs: &'a [u8],
}

pub async fn insert_report(pool: &PgPool, report: &NewReport<'_>) -> Result<String, ApiError> {
    sqlx::query_scalar(
        "insert into diagnostics.reports \
             (user_id, app_version, os, os_version, arch, webview_version, window_label, route, note, \
              meta, logs, logs_bytes) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) returning id",
    )
    .bind(report.user_id)
    .bind(report.app_version)
    .bind(report.os)
    .bind(report.os_version)
    .bind(report.arch)
    .bind(report.webview_version)
    .bind(report.window_label)
    .bind(report.route)
    .bind(report.note)
    .bind(report.meta)
    .bind(report.logs)
    .bind(i32::try_from(report.logs.len()).unwrap_or(i32::MAX))
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// How many reports `user_id` has sent since `since` — the hourly cap's count.
pub async fn count_since(
    pool: &PgPool,
    user_id: &str,
    since: DateTime<Utc>,
) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        "select count(*) from diagnostics.reports where user_id = $1 and created_at >= $2",
    )
    .bind(user_id)
    .bind(since)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

const SUMMARY_COLUMNS: &str = "r.id, r.created_at, u.cid as user_cid, u.display_name as user_display_name, \
     u.home_facility as user_artcc, r.app_version, r.os, r.os_version, r.arch, r.window_label, r.route";

pub async fn count_reports(pool: &PgPool) -> Result<i64, ApiError> {
    sqlx::query_scalar("select count(*) from diagnostics.reports")
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// A page of reports, newest first.
pub async fn list_reports(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<Vec<DiagnosticsReportSummary>, ApiError> {
    sqlx::query_as(&format!(
        "select {SUMMARY_COLUMNS}, r.note <> '' as has_note, r.logs_bytes \
         from diagnostics.reports r join identity.users u on u.id = r.user_id \
         order by r.created_at desc limit $1 offset $2"
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_report(pool: &PgPool, id: &str) -> Result<Option<DiagnosticsReport>, ApiError> {
    sqlx::query_as(&format!(
        "select {SUMMARY_COLUMNS}, r.webview_version, r.note, r.meta, r.logs_bytes \
         from diagnostics.reports r join identity.users u on u.id = r.user_id where r.id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The gzipped logs of one report.
pub async fn get_logs(pool: &PgPool, id: &str) -> Result<Option<Vec<u8>>, ApiError> {
    sqlx::query_scalar("select logs from diagnostics.reports where id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Deletes one report; `false` when there was none.
pub async fn delete_report(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    sqlx::query("delete from diagnostics.reports where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() > 0)
        .map_err(|_| ApiError::Internal)
}

/// Most rows one prune pass deletes; a backlog drains over later passes (as `audit::prune_audit_logs`).
const PRUNE_BATCH: i64 = 1_000;

/// Deletes reports older than `before`, up to [`PRUNE_BATCH`]. Reports carry megabytes of logs, so the
/// batch is far smaller than the audit log's.
pub async fn prune_reports(pool: &PgPool, before: DateTime<Utc>) -> Result<u64, ApiError> {
    sqlx::query(
        "delete from diagnostics.reports \
         where ctid in (select ctid from diagnostics.reports where created_at < $1 limit $2)",
    )
    .bind(before)
    .bind(PRUNE_BATCH)
    .execute(pool)
    .await
    .map(|r| r.rows_affected())
    .map_err(|_| ApiError::Internal)
}
