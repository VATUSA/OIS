//! User identity persistence for the login bootstrap.

use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    errors::ApiError,
    models::{AdminUserRow, UserSummary},
};

/// Match clause shared by the admin user browser's list + count queries. `$1` is the (trimmed) search
/// term; an empty term matches everyone who has signed in.
///
/// Users seeded by the VATUSA division pull who have never signed in (`last_login_at` null, #605) are
/// left out — the whole division would otherwise bury the people actually using OIS — **except on an
/// exact CID match**, so an admin can still find one to grant access before their first sign-in.
const USER_FILTER: &str = "u.cid is not null \
    and (u.last_login_at is not null or cast(u.cid as text) = $1) \
    and ( \
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
            coalesce(array_agg(distinct ur.role_name) filter (where ur.role_name is not null), '{{}}') as roles, \
            coalesce(array_agg(distinct ur.role_name || coalesce(':' || ur.artcc_id, '')) \
                filter (where ur.role_name is not null), '{{}}') as scoped_roles \
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
    pub first_sign_in: bool,
}

/// Fuzzy user search by display/full name (substring) or CID (prefix). Exact CID
/// matches sort first, then by name. `query` is assumed non-empty and trimmed.
pub async fn search_users(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<UserSummary>, ApiError> {
    sqlx::query_as::<_, UserSummary>(
        // Same rule as `USER_FILTER`: never-signed-in seeded users only on an exact CID (#605).
        "select cid, display_name, rating from identity.users \
         where cid is not null \
           and (last_login_at is not null or cast(cid as text) = $1) \
           and ( \
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
        with prev as (select last_login_at from identity.users where cid = $2)
        insert into identity.users as u
            (id, cid, email, full_name, display_name, rating, last_login_at)
        values ($1, $2, $3, $4, $5, $6, now())
        on conflict (cid) do update
        set email = excluded.email,
            last_login_at = now(),
            full_name = excluded.full_name,
            display_name = excluded.display_name,
            -- Don't let a login that couldn't read the rating (or a source that omits it) wipe a
            -- rating the VATUSA sync already established; only overwrite with a non-null value.
            rating = coalesce(excluded.rating, u.rating),
            updated_at = now()
        -- A first sign-in, not a first *insert*: the daily VATUSA pull (#605) seeds rows for people
        -- who have never signed in, so their first sign-in is an update. Keying on the insert would
        -- skip their baseline `USER` group. No previous row, or one never signed in, is a first.
        returning id, (select last_login_at from prev) is null as first_sign_in
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
        first_sign_in: row.1,
    })
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    /// One user who has signed in, and one seeded by the VATUSA division pull who never has (#605).
    async fn seed(pool: &PgPool) {
        sqlx::query(
            "insert into identity.users (full_name, display_name, cid, last_login_at) values \
                 ('Signed In', 'Zed Signed', 1605601, now()), \
                 ('Seeded Only', 'Zed Seeded', 1605602, null)",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// AC3: browsing and name search show the people actually using OIS — the whole division, seeded
    /// by the pull, would otherwise bury them — but an exact CID still finds a seeded user, so an admin
    /// can grant access before their first sign-in.
    #[sqlx::test]
    async fn seeded_users_are_hidden_from_browsing_but_found_by_exact_cid(pool: PgPool) {
        seed(&pool).await;

        let browse: Vec<i64> = list_users(&pool, "Zed", 50, 0)
            .await
            .unwrap()
            .into_iter()
            .map(|u| u.cid)
            .collect();
        assert_eq!(browse, [1_605_601]);
        assert_eq!(count_users(&pool, "Zed").await.unwrap(), 1);
        assert_eq!(count_users(&pool, "").await.unwrap(), 1);

        let found: Vec<i64> = list_users(&pool, "1605602", 50, 0)
            .await
            .unwrap()
            .into_iter()
            .map(|u| u.cid)
            .collect();
        assert_eq!(found, [1_605_602], "an exact CID reaches a seeded user");

        let search = |q: &'static str| {
            let pool = pool.clone();
            async move {
                search_users(&pool, q, 10)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|u| u.cid)
                    .collect::<Vec<_>>()
            }
        };
        assert_eq!(search("Zed").await, [1_605_601]);
        assert_eq!(
            search("160560").await,
            [1_605_601],
            "a CID prefix is browsing, not a match"
        );
        assert_eq!(search("1605602").await, [1_605_602]);
    }

    /// A seeded user's first sign-in is an *update* of the row the pull made, not an insert — and it
    /// must still count as their first, or they never get the baseline `USER` group.
    #[sqlx::test]
    async fn a_seeded_users_first_sign_in_is_their_first(pool: PgPool) {
        seed(&pool).await;
        let sign_in = |cid: i64| {
            let pool = pool.clone();
            async move {
                let mut tx = pool.begin().await.unwrap();
                let user = upsert_login_user(
                    &mut tx,
                    &uuid::Uuid::new_v4().to_string(),
                    cid,
                    &format!("{cid}@example.test"),
                    "Real Name",
                    "Real Name",
                    None,
                )
                .await
                .unwrap();
                tx.commit().await.unwrap();
                user.first_sign_in
            }
        };

        assert!(sign_in(1_605_602).await, "seeded, never signed in");
        assert!(!sign_in(1_605_602).await, "and only the first time");
        assert!(!sign_in(1_605_601).await, "already signed in before");
        assert!(sign_in(1_605_603).await, "a brand-new user, as before");
    }
}
