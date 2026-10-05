//! Service-account persistence (machine clients — the Discord bot). Bearer secrets are
//! stored only as SHA-256 hashes; the plaintext token is returned once at issue time.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{ApiKeyPermissionBody, CredentialUsageBody, ServiceAccountBody},
    repos::access as access_repo,
};

/// Set or clear one account's rate limit override (#611). `false` if there is no such account.
pub async fn set_rate_limit(
    pool: &PgPool,
    id: &str,
    per_min: Option<i32>,
) -> Result<bool, ApiError> {
    sqlx::query(
        "update access.service_accounts set rate_limit_per_min = $2, updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(per_min)
    .execute(pool)
    .await
    .map(|r| r.rows_affected() > 0)
    .map_err(|_| ApiError::Internal)
}

/// Days without use (or, never used, since issue) after which a live credential reads as stale.
const STALE_AFTER_DAYS: i32 = 30;

#[derive(sqlx::FromRow)]
struct ServiceAccountRow {
    id: String,
    key: String,
    name: String,
    description: Option<String>,
    status: String,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    stale: bool,
    created_at: DateTime<Utc>,
    rate_limit_per_min: Option<i32>,
    requests_this_hour: i64,
    requests_last_day: i64,
    refused_last_day: i64,
}

async fn row_into_body(
    pool: &PgPool,
    row: ServiceAccountRow,
) -> Result<ServiceAccountBody, ApiError> {
    let roles = access_repo::fetch_service_account_role_names(pool, &row.id).await?;
    let permissions = fetch_permissions(pool, &row.id).await?;
    Ok(ServiceAccountBody {
        id: row.id,
        key: row.key,
        name: row.name,
        description: row.description,
        status: row.status,
        roles,
        permissions,
        last_used_at: row.last_used_at,
        expires_at: row.expires_at,
        stale: row.stale,
        created_at: row.created_at,
        rate_limit_per_min: row.rate_limit_per_min,
        usage: CredentialUsageBody {
            requests_this_hour: row.requests_this_hour,
            requests_last_day: row.requests_last_day,
            refused_last_day: row.refused_last_day,
        },
    })
}

/// The account plus its live credential — rotate revokes the old one, so there is at most one. `$1`
/// is the stale threshold in days. `stale` is false with no live credential: a disabled account has
/// nothing left to rotate.
const SELECT: &str = "select sa.id, sa.key, sa.name, sa.description, sa.status, \
    c.last_used_at, c.expires_at, \
    coalesce(coalesce(c.last_used_at, c.created_at) < now() - make_interval(days => $1), false) \
        as stale, \
    sa.created_at, sa.rate_limit_per_min, \
    (select coalesce(sum(requests), 0) from access.credential_usage cu \
     where cu.kind = 'service_account' and cu.credential_id = sa.id \
       and cu.hour >= date_trunc('hour', now()))::bigint as requests_this_hour, \
    (select coalesce(sum(requests), 0) from access.credential_usage cu \
     where cu.kind = 'service_account' and cu.credential_id = sa.id \
       and cu.hour > now() - interval '24 hours')::bigint as requests_last_day, \
    (select coalesce(sum(refused), 0) from access.credential_usage cu \
     where cu.kind = 'service_account' and cu.credential_id = sa.id \
       and cu.hour > now() - interval '24 hours')::bigint as refused_last_day \
    from access.service_accounts sa \
    left join lateral (select last_used_at, expires_at, created_at \
        from access.service_account_credentials \
        where service_account_id = sa.id and revoked_at is null \
        order by created_at desc limit 1) c on true";

pub async fn list_service_accounts(pool: &PgPool) -> Result<Vec<ServiceAccountBody>, ApiError> {
    let rows =
        sqlx::query_as::<_, ServiceAccountRow>(&format!("{SELECT} order by sa.created_at desc"))
            .bind(STALE_AFTER_DAYS)
            .fetch_all(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_into_body(pool, row).await?);
    }
    Ok(out)
}

pub async fn get_service_account(
    pool: &PgPool,
    id: &str,
) -> Result<Option<ServiceAccountBody>, ApiError> {
    let row = sqlx::query_as::<_, ServiceAccountRow>(&format!("{SELECT} where sa.id = $2"))
        .bind(STALE_AFTER_DAYS)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    match row {
        Some(row) => Ok(Some(row_into_body(pool, row).await?)),
        None => Ok(None),
    }
}

/// An account's direct grants (#584), as `(permission, artcc_id)`; `artcc_id = None` is national.
pub async fn fetch_permissions(
    pool: &PgPool,
    id: &str,
) -> Result<Vec<ApiKeyPermissionBody>, ApiError> {
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "select permission_name, artcc_id from access.service_account_permissions \
         where service_account_id = $1 order by permission_name, artcc_id nulls first",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(permission, artcc_id)| ApiKeyPermissionBody {
            permission,
            artcc_id,
        })
        .collect())
}

async fn ensure_exists(pool: &PgPool, id: &str) -> Result<(), ApiError> {
    sqlx::query_scalar::<_, String>("select id from access.service_accounts where id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)?
        .map(|_| ())
        .ok_or(ApiError::NotFound)
}

async fn insert_credential(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
    secret_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into access.service_account_credentials \
         (service_account_id, credential_type, secret_hash, expires_at) \
         values ($1, 'bearer_token', $2, $3)",
    )
    .bind(id)
    .bind(secret_hash)
    .bind(expires_at)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Creates a service account and stores the first credential's hash. Returns the id.
pub async fn create_service_account(
    pool: &PgPool,
    key: &str,
    name: &str,
    description: Option<&str>,
    secret_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<String, ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let id = sqlx::query_scalar::<_, String>(
        "insert into access.service_accounts (key, name, description) values ($1, $2, $3) returning id",
    )
    .bind(key)
    .bind(name)
    .bind(description)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    insert_credential(&mut tx, &id, secret_hash, expires_at).await?;

    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(id)
}

/// Revokes all active credentials and issues a new one. Errors NotFound if absent.
pub async fn rotate_credential(
    pool: &PgPool,
    id: &str,
    secret_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    ensure_exists(pool, id).await?;

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query(
        "update access.service_account_credentials set revoked_at = now() \
         where service_account_id = $1 and revoked_at is null",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    insert_credential(&mut tx, id, secret_hash, expires_at).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Disables the account and revokes its credentials. Errors NotFound if absent.
pub async fn disable_service_account(pool: &PgPool, id: &str) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let result =
        sqlx::query("update access.service_accounts set status = 'disabled' where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    sqlx::query(
        "update access.service_account_credentials set revoked_at = now() \
         where service_account_id = $1 and revoked_at is null",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Replaces the account's roles (national scope). Errors NotFound if absent.
pub async fn set_roles(pool: &PgPool, id: &str, role_names: &[String]) -> Result<(), ApiError> {
    ensure_exists(pool, id).await?;

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query("delete from access.service_account_roles where service_account_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for role_name in role_names {
        sqlx::query(
            "insert into access.service_account_roles (service_account_id, role_name) values ($1, $2)",
        )
        .bind(id)
        .bind(role_name)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Replaces the account's direct `(permission, artcc_id)` grants (#584). Errors NotFound if absent.
/// The caller has already validated them against the acting admin's authority.
pub async fn set_permissions(
    pool: &PgPool,
    id: &str,
    grants: &[(String, Option<String>)],
) -> Result<(), ApiError> {
    ensure_exists(pool, id).await?;

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query("delete from access.service_account_permissions where service_account_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for (permission_name, artcc_id) in grants {
        // A repeated grant is the same grant, not an error.
        sqlx::query(
            "insert into access.service_account_permissions \
             (service_account_id, permission_name, artcc_id) values ($1, $2, $3) \
             on conflict do nothing",
        )
        .bind(id)
        .bind(permission_name)
        .bind(artcc_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}
