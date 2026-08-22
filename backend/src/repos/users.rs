//! User identity persistence for the login bootstrap.

use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    errors::ApiError,
    models::{AdminUserRow, UserSummary},
};

/// Match clause shared by the admin user browser's list + count queries. `$1` is the (trimmed) search
/// term; an empty term matches everyone.
const USER_FILTER: &str = "u.cid is not null and ( \
    $1 = '' \
    or u.display_name ilike '%' || $1 || '%' \
    or u.full_name ilike '%' || $1 || '%' \
    or cast(u.cid as text) like $1 || '%' )";

/// One page of all OIS users (optionally filtered by `q`), each with the distinct role names they
/// hold across any scope. Ordered by name.
pub async fn list_users(
    pool: &PgPool,
    q: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<AdminUserRow>, ApiError> {
    sqlx::query_as::<_, AdminUserRow>(&format!(
        "select u.cid, u.display_name, u.rating, \
            coalesce(array_agg(distinct ur.role_name) filter (where ur.role_name is not null), '{{}}') as roles \
         from identity.users u \
         left join access.user_roles ur on ur.user_id = u.id \
         where {USER_FILTER} \
         group by u.cid, u.display_name, u.rating \
         order by u.display_name asc \
         limit $2 offset $3"
    ))
    .bind(q)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Total users matching `q` (for pagination).
pub async fn count_users(pool: &PgPool, q: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, i64>(&format!(
        "select count(*) from identity.users u where {USER_FILTER}"
    ))
    .bind(q)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub struct LoginUser {
    pub id: String,
    pub was_new_user: bool,
}

/// Fuzzy user search by display/full name (substring) or CID (prefix). Exact CID
/// matches sort first, then by name. `query` is assumed non-empty and trimmed.
pub async fn search_users(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<UserSummary>, ApiError> {
    sqlx::query_as::<_, UserSummary>(
        "select cid, display_name, rating from identity.users \
         where cid is not null and ( \
             display_name ilike '%' || $1 || '%' \
             or full_name ilike '%' || $1 || '%' \
             or cast(cid as text) like $1 || '%' \
         ) \
         order by (cast(cid as text) = $1) desc, display_name asc \
         limit $2",
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Upserts the identity row for a logging-in user. `new_id` is used only when the row
/// is created. `was_new_user` is derived from the `xmax = 0` insert/update marker.
pub async fn upsert_login_user(
    tx: &mut Transaction<'_, Postgres>,
    new_id: &str,
    cid: i64,
    email: &str,
    full_name: &str,
    display_name: &str,
    rating: Option<&str>,
) -> Result<LoginUser, ApiError> {
    let row = sqlx::query_as::<_, (String, bool)>(
        r#"
        insert into identity.users as u (id, cid, email, full_name, display_name, rating)
        values ($1, $2, $3, $4, $5, $6)
        on conflict (cid) do update
        set email = excluded.email,
            full_name = excluded.full_name,
            display_name = excluded.display_name,
            -- Don't let a login that couldn't read the rating (or a source that omits it) wipe a
            -- rating the VATUSA sync already established; only overwrite with a non-null value.
            rating = coalesce(excluded.rating, u.rating),
            updated_at = now()
        returning id, (xmax = 0) as was_new_user
        "#,
    )
    .bind(new_id)
    .bind(cid)
    .bind(email)
    .bind(full_name)
    .bind(display_name)
    .bind(rating)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(LoginUser {
        id: row.0,
        was_new_user: row.1,
    })
}
