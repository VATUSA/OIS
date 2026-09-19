//! Session persistence for the auth handlers.
//!
//! Web and desktop sessions live in the same `identity.sessions` table and differ only in `kind`
//! and in how the token reaches us — a cookie on the web, an `Authorization: Bearer ois_dsk_…`
//! header on the desktop. Reading a session is therefore shared:
//! [`crate::repos::access::find_current_user_by_session_token`] serves both.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;

/// How long a login lasts, web or desktop. Mirrors the cookie `Max-Age` the handler sets.
const SESSION_TTL: &str = "30 days";

/// One-time desktop auth codes are redeemed within a second or two — the app is already blocked on
/// its loopback listener when the code is minted — so the window is deliberately tight.
const AUTH_CODE_TTL_SECONDS: i64 = 60;

/// Inserts a new 30-day login session for the web (cookie) flow.
pub async fn insert_session(
    pool: &PgPool,
    session_token: &str,
    user_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(&format!(
        r#"
        insert into identity.sessions (session_token, user_id, kind, expires_at)
        values ($1, $2, 'web', now() + interval '{SESSION_TTL}')
        "#
    ))
    .bind(session_token)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(())
}

/// Revokes a session by token (hard delete). Used by logout for both web and desktop.
pub async fn delete_session(pool: &PgPool, session_token: &str) -> Result<(), ApiError> {
    sqlx::query("delete from identity.sessions where session_token = $1")
        .bind(session_token)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;

    Ok(())
}

/// Inserts a desktop session, returning when it expires so the app can schedule a refresh.
pub async fn insert_desktop_session(
    pool: &PgPool,
    session_token: &str,
    user_id: &str,
) -> Result<DateTime<Utc>, ApiError> {
    sqlx::query_scalar::<_, DateTime<Utc>>(&format!(
        r#"
        insert into identity.sessions (session_token, user_id, kind, expires_at)
        values ($1, $2, 'desktop', now() + interval '{SESSION_TTL}')
        returning expires_at
        "#
    ))
    .bind(session_token)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Records a one-time code for the desktop app to exchange. Only its hash is stored.
pub async fn insert_desktop_auth_code(
    pool: &PgPool,
    code_hash: &str,
    user_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        insert into identity.desktop_auth_codes (code_hash, user_id, expires_at)
        values ($1, $2, now() + make_interval(secs => $3))
        "#,
    )
    .bind(code_hash)
    .bind(user_id)
    .bind(AUTH_CODE_TTL_SECONDS as f64)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(())
}

/// Redeems a one-time code, returning the user it was minted for.
///
/// Consuming and checking are the same statement, so two concurrent exchanges of one code cannot
/// both succeed — the second matches no row. `None` covers all of: unknown, already consumed, and
/// expired, which the caller reports identically so a probe learns nothing from the difference.
pub async fn consume_desktop_auth_code(
    pool: &PgPool,
    code_hash: &str,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        r#"
        update identity.desktop_auth_codes
        set consumed_at = now()
        where code_hash = $1
          and consumed_at is null
          and expires_at > now()
        returning user_id
        "#,
    )
    .bind(code_hash)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Rotates a desktop session: the old token is destroyed and a fresh one takes its place in the
/// same statement, so a leaked token stops working the moment the app refreshes.
///
/// Returns `None` when the presented token is not a live **desktop** session — a web cookie token
/// is deliberately not rotatable here, so a stolen cookie cannot be upgraded into a long-lived
/// keychain credential.
pub async fn rotate_desktop_session(
    pool: &PgPool,
    old_token: &str,
    new_token: &str,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    sqlx::query_scalar::<_, DateTime<Utc>>(&format!(
        r#"
        with rotated as (
            delete from identity.sessions
            where session_token = $1
              and kind = 'desktop'
              and revoked_at is null
              and expires_at > now()
            returning user_id
        )
        insert into identity.sessions (session_token, user_id, kind, expires_at)
        select $2, user_id, 'desktop', now() + interval '{SESSION_TTL}'
        from rotated
        returning expires_at
        "#
    ))
    .bind(old_token)
    .bind(new_token)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos::access::{find_current_user_by_session_token, sha256_hex};

    /// A user to hang sessions off. `identity.users` is the only prerequisite for these paths.
    async fn user(pool: &PgPool, cid: i64) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into identity.users (cid, full_name, display_name) values ($1, $2, $2) returning id",
        )
        .bind(cid)
        .bind(format!("Test {cid}"))
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn expire_code(pool: &PgPool, code_hash: &str) {
        sqlx::query("update identity.desktop_auth_codes set expires_at = now() - interval '1 second' where code_hash = $1")
            .bind(code_hash)
            .execute(pool)
            .await
            .unwrap();
    }

    #[sqlx::test]
    async fn a_code_can_be_exchanged_once(pool: PgPool) {
        let user_id = user(&pool, 1001).await;
        let hash = sha256_hex("the-code");
        insert_desktop_auth_code(&pool, &hash, &user_id)
            .await
            .unwrap();

        assert_eq!(
            consume_desktop_auth_code(&pool, &hash).await.unwrap(),
            Some(user_id),
            "first exchange returns the user the code was minted for"
        );
        assert_eq!(
            consume_desktop_auth_code(&pool, &hash).await.unwrap(),
            None,
            "a replayed code must not mint a second session"
        );
    }

    #[sqlx::test]
    async fn an_expired_code_is_refused(pool: PgPool) {
        let user_id = user(&pool, 1002).await;
        let hash = sha256_hex("stale");
        insert_desktop_auth_code(&pool, &hash, &user_id)
            .await
            .unwrap();
        expire_code(&pool, &hash).await;

        assert_eq!(consume_desktop_auth_code(&pool, &hash).await.unwrap(), None);
    }

    #[sqlx::test]
    async fn an_unknown_code_is_refused(pool: PgPool) {
        assert_eq!(
            consume_desktop_auth_code(&pool, &sha256_hex("never-minted"))
                .await
                .unwrap(),
            None
        );
    }

    #[sqlx::test]
    async fn a_desktop_session_authenticates_as_the_same_user_a_cookie_would(pool: PgPool) {
        let user_id = user(&pool, 1003).await;
        insert_desktop_session(&pool, "ois_dsk_abc", &user_id)
            .await
            .unwrap();

        let resolved = find_current_user_by_session_token(&pool, "ois_dsk_abc")
            .await
            .unwrap()
            .expect("a desktop session resolves through the shared session lookup");
        assert_eq!(resolved.id, user_id);
        assert_eq!(resolved.cid, 1003);
    }

    #[sqlx::test]
    async fn refresh_rotates_and_kills_the_old_token(pool: PgPool) {
        let user_id = user(&pool, 1004).await;
        insert_desktop_session(&pool, "ois_dsk_old", &user_id)
            .await
            .unwrap();

        rotate_desktop_session(&pool, "ois_dsk_old", "ois_dsk_new")
            .await
            .unwrap()
            .expect("a live desktop session rotates");

        assert!(
            find_current_user_by_session_token(&pool, "ois_dsk_new")
                .await
                .unwrap()
                .is_some(),
            "the new token works"
        );
        assert!(
            find_current_user_by_session_token(&pool, "ois_dsk_old")
                .await
                .unwrap()
                .is_none(),
            "the old token stops working the moment it is rotated"
        );
    }

    #[sqlx::test]
    async fn refresh_refuses_a_web_session(pool: PgPool) {
        let user_id = user(&pool, 1005).await;
        insert_session(&pool, "web-cookie-token", &user_id)
            .await
            .unwrap();

        assert!(
            rotate_desktop_session(&pool, "web-cookie-token", "ois_dsk_new")
                .await
                .unwrap()
                .is_none(),
            "a stolen browser cookie must not be upgradeable to a desktop token"
        );
        assert!(
            find_current_user_by_session_token(&pool, "web-cookie-token")
                .await
                .unwrap()
                .is_some(),
            "and the refused rotation must not have deleted the web session"
        );
    }

    #[sqlx::test]
    async fn refresh_refuses_an_unknown_token(pool: PgPool) {
        assert!(
            rotate_desktop_session(&pool, "ois_dsk_nope", "ois_dsk_new")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[sqlx::test]
    async fn logout_deletes_a_desktop_session(pool: PgPool) {
        let user_id = user(&pool, 1006).await;
        insert_desktop_session(&pool, "ois_dsk_bye", &user_id)
            .await
            .unwrap();

        delete_session(&pool, "ois_dsk_bye").await.unwrap();

        assert!(
            find_current_user_by_session_token(&pool, "ois_dsk_bye")
                .await
                .unwrap()
                .is_none()
        );
    }
}
