//! User identity persistence for the login bootstrap.

use sqlx::{Postgres, Transaction};

use crate::errors::ApiError;

pub struct LoginUser {
    pub id: String,
    pub was_new_user: bool,
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
        insert into identity.users (id, cid, email, full_name, display_name, rating)
        values ($1, $2, $3, $4, $5, $6)
        on conflict (cid) do update
        set email = excluded.email,
            full_name = excluded.full_name,
            display_name = excluded.display_name,
            rating = excluded.rating,
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
