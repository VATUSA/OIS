//! Service-account persistence (machine clients — the Discord bot). Bearer secrets are
//! stored only as SHA-256 hashes; the plaintext token is returned once at issue time.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{errors::ApiError, models::ServiceAccountBody, repos::access as access_repo};

#[derive(sqlx::FromRow)]
struct ServiceAccountRow {
    id: String,
    key: String,
    name: String,
    description: Option<String>,
    status: String,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

async fn row_into_body(
    pool: &PgPool,
    row: ServiceAccountRow,
) -> Result<ServiceAccountBody, ApiError> {
    let roles = access_repo::fetch_service_account_role_names(pool, &row.id).await?;
    Ok(ServiceAccountBody {
        id: row.id,
        key: row.key,
        name: row.name,
        description: row.description,
        status: row.status,
        roles,
        last_used_at: row.last_used_at,
        created_at: row.created_at,
    })
}

const SELECT: &str = "select sa.id, sa.key, sa.name, sa.description, sa.status, \
    (select max(last_used_at) from access.service_account_credentials c \
     where c.service_account_id = sa.id and c.revoked_at is null) as last_used_at, \
    sa.created_at from access.service_accounts sa";

pub async fn list_service_accounts(pool: &PgPool) -> Result<Vec<ServiceAccountBody>, ApiError> {
    let rows =
        sqlx::query_as::<_, ServiceAccountRow>(&format!("{SELECT} order by sa.created_at desc"))
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
    let row = sqlx::query_as::<_, ServiceAccountRow>(&format!("{SELECT} where sa.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    match row {
        Some(row) => Ok(Some(row_into_body(pool, row).await?)),
        None => Ok(None),
    }
}

/// Creates a service account and stores the first credential's hash. Returns the id.
pub async fn create_service_account(
    pool: &PgPool,
    key: &str,
    name: &str,
    description: Option<&str>,
    secret_hash: &str,
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

    sqlx::query(
        "insert into access.service_account_credentials (service_account_id, credential_type, secret_hash) \
         values ($1, 'bearer_token', $2)",
    )
    .bind(&id)
    .bind(secret_hash)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(id)
}

/// Revokes all active credentials and issues a new one. Errors NotFound if absent.
pub async fn rotate_credential(pool: &PgPool, id: &str, secret_hash: &str) -> Result<(), ApiError> {
    let exists =
        sqlx::query_scalar::<_, String>("select id from access.service_accounts where id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query(
        "update access.service_account_credentials set revoked_at = now() \
         where service_account_id = $1 and revoked_at is null",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    sqlx::query(
        "insert into access.service_account_credentials (service_account_id, credential_type, secret_hash) \
         values ($1, 'bearer_token', $2)",
    )
    .bind(id)
    .bind(secret_hash)
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;
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
    let exists =
        sqlx::query_scalar::<_, String>("select id from access.service_accounts where id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }

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
