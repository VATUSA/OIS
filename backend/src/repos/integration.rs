//! Discord integration persistence: the outbound-job queue (backend enqueues in its own tx; the bot
//! leases → acks) and the guild config (logical name → snowflake maps). The bot owns no data.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    errors::ApiError,
    models::{
        DiscordConfigBody, DiscordGuildChannel, DiscordGuildConfigBody, DiscordGuildRole,
        DiscordGuildSnapshotBody, DiscordMapEntry, OutboundJobBody, UpsertDiscordConfigRequest,
    },
};

/// Retry backoff cap and the max attempts before a job is parked as `failed`.
const MAX_ATTEMPTS: i32 = 8;

/// Enqueue an outbound job **inside the caller's transaction**, so the side-effect is atomic with
/// the state change that triggered it (no job without the change, no change without the job).
pub async fn enqueue_job(
    tx: &mut Transaction<'_, Postgres>,
    job_type: &str,
    payload: &Value,
    subject_type: Option<&str>,
    subject_id: Option<&str>,
) -> Result<String, ApiError> {
    let payload = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    sqlx::query_scalar::<_, String>(
        "insert into integration.outbound_jobs (job_type, payload, subject_type, subject_id) \
         values ($1, $2::jsonb, $3, $4) returning id",
    )
    .bind(job_type)
    .bind(payload)
    .bind(subject_type)
    .bind(subject_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)
}

#[derive(sqlx::FromRow)]
struct LeaseRow {
    id: String,
    job_type: String,
    payload: String,
    subject_type: Option<String>,
    subject_id: Option<String>,
    attempt_count: i32,
    created_at: DateTime<Utc>,
}

/// Atomically lease up to `limit` due jobs: flip them `pending → in_progress`, bump `attempt_count`,
/// and hand them over. `for update skip locked` lets multiple bot instances lease without collisions.
pub async fn lease_jobs(pool: &PgPool, limit: i64) -> Result<Vec<OutboundJobBody>, ApiError> {
    let rows = sqlx::query_as::<_, LeaseRow>(
        "update integration.outbound_jobs j \
         set status = 'in_progress', attempt_count = j.attempt_count + 1, last_attempt_at = now() \
         from ( \
             select id from integration.outbound_jobs \
             where status = 'pending' and next_attempt_at <= now() \
             order by next_attempt_at for update skip locked limit $1 \
         ) d \
         where j.id = d.id \
         returning j.id, j.job_type, j.payload::text as payload, j.subject_type, j.subject_id, \
                   j.attempt_count, j.created_at",
    )
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(rows
        .into_iter()
        .map(|r| OutboundJobBody {
            id: r.id,
            job_type: r.job_type,
            payload: serde_json::from_str(&r.payload).unwrap_or(Value::Null),
            subject_type: r.subject_type,
            subject_id: r.subject_id,
            attempt_count: r.attempt_count,
            created_at: r.created_at,
        })
        .collect())
}

/// Acknowledge a leased job. Success → `succeeded` (+ `result`). Failure → retry with linear backoff,
/// or `failed` once `MAX_ATTEMPTS` is reached. Returns false if the id doesn't exist.
pub async fn ack_job(
    pool: &PgPool,
    id: &str,
    success: bool,
    result: Option<&Value>,
    error: Option<&str>,
) -> Result<bool, ApiError> {
    let res = if success {
        let result =
            result.map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".to_string()));
        sqlx::query(
            "update integration.outbound_jobs \
             set status = 'succeeded', result = $2::jsonb, error = null where id = $1",
        )
        .bind(id)
        .bind(result)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?
    } else {
        sqlx::query(
            "update integration.outbound_jobs \
             set status = case when attempt_count >= $2 then 'failed' else 'pending' end, \
                 next_attempt_at = now() + (interval '30 seconds' * least(attempt_count, 10)), \
                 error = $3 \
             where id = $1",
        )
        .bind(id)
        .bind(MAX_ATTEMPTS)
        .bind(error)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?
    };
    Ok(res.rows_affected() > 0)
}

/// Resolve a logical channel name to its Discord snowflake for the configured guild (None if
/// unmapped / no config). Callers skip enqueuing a Discord job when there's nowhere to post.
pub async fn channel_id(pool: &PgPool, name: &str) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select ch.channel_id from integration.discord_channels ch \
         join integration.discord_configs c on c.id = ch.config_id \
         where ch.name = $1 order by c.created_at limit 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Resolve a logical role name to its Discord snowflake for the configured guild (None if unmapped).
pub async fn role_id(pool: &PgPool, name: &str) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select r.role_id from integration.discord_roles r \
         join integration.discord_configs c on c.id = r.config_id \
         where r.name = $1 order by c.created_at limit 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Discord user ids of a facility's EC(s): OIS users holding the `EC` role scoped to that ARTCC (set
/// via Access Control) who have a VATUSA-linked Discord. Empty if none assigned or none linked.
pub async fn ec_discord_ids(pool: &PgPool, facility: &str) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select m.external_id \
         from access.user_roles ur \
         join integration.external_sync_mappings m \
           on m.system_code = 'discord' and m.entity_type = 'user' and m.local_id = ur.user_id \
         where ur.role_name = 'EC' and ur.artcc_id = $1",
    )
    .bind(facility)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The `result` payload of the most recent succeeded job for a subject + type — used to recover ids
/// the bot returned on ack (e.g. the posted message id, needed by a follow-up job).
pub async fn succeeded_job_result(
    pool: &PgPool,
    subject_type: &str,
    subject_id: &str,
    job_type: &str,
) -> Result<Option<Value>, ApiError> {
    let text = sqlx::query_scalar::<_, Option<String>>(
        "select result::text from integration.outbound_jobs \
         where subject_type = $1 and subject_id = $2 and job_type = $3 and status = 'succeeded' \
         order by created_at desc limit 1",
    )
    .bind(subject_type)
    .bind(subject_id)
    .bind(job_type)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(text.flatten().and_then(|t| serde_json::from_str(&t).ok()))
}

// --- Discord link lookups (external_sync_mappings) -----------------------------------------------
// The mapping is populated from VATUSA during member sync (see repos::vatusa); here we only read it.

/// The Discord account linked to an OIS user: `(discord_user_id, metadata)`, or `None`.
pub async fn get_discord_link(
    pool: &PgPool,
    user_id: &str,
) -> Result<Option<(String, Value)>, ApiError> {
    let row = sqlx::query_as::<_, (String, Value)>(
        "select external_id, coalesce(metadata, '{}'::jsonb) from integration.external_sync_mappings \
         where system_code = 'discord' and entity_type = 'user' and local_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(row)
}

/// The OIS user id linked to a Discord account, or `None` — used by the bot's interaction callbacks
/// to act on behalf of the clicking user.
pub async fn find_user_by_discord_id(
    pool: &PgPool,
    discord_id: &str,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select local_id from integration.external_sync_mappings \
         where system_code = 'discord' and entity_type = 'user' and external_id = $1",
    )
    .bind(discord_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

// --- config (single guild for the first cut) -----------------------------------------------------

async fn map_entries(
    pool: &PgPool,
    table: &str,
    id_col: &str,
    config_id: &str,
) -> Result<Vec<DiscordMapEntry>, ApiError> {
    // `table`/`id_col` are trusted internal constants (never user input); values are bound.
    let sql = format!(
        "select name, {id_col} as id from integration.{table} where config_id = $1 order by name"
    );
    sqlx::query_as::<_, DiscordMapEntry>(&sql)
        .bind(config_id)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// The full Discord config: every configured guild + its maps, plus the bot's guild snapshot for
/// the editor's dropdowns. Empty `guilds` when nothing's been configured yet.
pub async fn get_config(pool: &PgPool) -> Result<DiscordConfigBody, ApiError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "select id, name, guild_id from integration.discord_configs order by created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let mut guilds = Vec::with_capacity(rows.len());
    for (id, name, guild_id) in rows {
        guilds.push(DiscordGuildConfigBody {
            channels: map_entries(pool, "discord_channels", "channel_id", &id).await?,
            roles: map_entries(pool, "discord_roles", "role_id", &id).await?,
            id: Some(id),
            name,
            guild_id,
        });
    }
    Ok(DiscordConfigBody {
        guilds,
        available: get_guild_snapshots(pool).await?,
    })
}

/// Fully replace the configured guilds + their maps, in one transaction. Config-row ids are not
/// stable across saves (nothing references them but their own cascade-deleted maps).
pub async fn upsert_config(
    pool: &PgPool,
    req: &UpsertDiscordConfigRequest,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query("delete from integration.discord_configs")
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for g in &req.guilds {
        if g.guild_id.trim().is_empty() {
            continue;
        }
        let config_id = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id) values ($1, $2) returning id",
        )
        .bind(g.name.trim())
        .bind(g.guild_id.trim())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
        replace_map(
            &mut tx,
            "discord_channels",
            "channel_id",
            &config_id,
            &g.channels,
        )
        .await?;
        replace_map(&mut tx, "discord_roles", "role_id", &config_id, &g.roles).await?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

// --- event-thread message template (singleton row; see migration 0065) ---------------------------

/// The configured event-thread message body (placeholders substituted by the bot at render time).
pub async fn get_event_thread_template(pool: &PgPool) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select body from integration.event_thread_template where id = 'default'",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn set_event_thread_template(pool: &PgPool, body: &str) -> Result<String, ApiError> {
    sqlx::query(
        "insert into integration.event_thread_template (id, body) values ('default', $1) \
         on conflict (id) do update set body = excluded.body",
    )
    .bind(body)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    get_event_thread_template(pool).await
}

// --- guild snapshot (channels + roles the bot sees; drives the config dropdowns) ------------------

/// Every guild the bot is in, with its channels + roles.
pub async fn get_guild_snapshots(pool: &PgPool) -> Result<Vec<DiscordGuildSnapshotBody>, ApiError> {
    let guilds = sqlx::query_as::<_, (String, String)>(
        "select guild_id, name from integration.discord_guilds order by name",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let mut out = Vec::with_capacity(guilds.len());
    for (guild_id, name) in guilds {
        let channels = sqlx::query_as::<_, DiscordGuildChannel>(
            "select channel_id as id, name, kind, parent_id, position \
             from integration.discord_guild_channels where guild_id = $1 order by position, name",
        )
        .bind(&guild_id)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
        let roles = sqlx::query_as::<_, DiscordGuildRole>(
            "select role_id as id, name, managed, position \
             from integration.discord_guild_roles where guild_id = $1 order by position desc, name",
        )
        .bind(&guild_id)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
        out.push(DiscordGuildSnapshotBody {
            guild_id,
            name,
            channels,
            roles,
        });
    }
    Ok(out)
}

/// Replace the whole guild snapshot with what the bot just pushed (full replace across all guilds).
pub async fn replace_guild_snapshots(
    pool: &PgPool,
    guilds: &[DiscordGuildSnapshotBody],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    // Cascade clears channels/roles.
    sqlx::query("delete from integration.discord_guilds")
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for g in guilds {
        sqlx::query(
            "insert into integration.discord_guilds (guild_id, name, synced_at) values ($1, $2, now())",
        )
        .bind(&g.guild_id)
        .bind(&g.name)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
        for ch in &g.channels {
            sqlx::query(
                "insert into integration.discord_guild_channels \
                 (guild_id, channel_id, name, kind, parent_id, position) \
                 values ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&g.guild_id)
            .bind(&ch.id)
            .bind(&ch.name)
            .bind(&ch.kind)
            .bind(&ch.parent_id)
            .bind(ch.position)
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
        }
        for r in &g.roles {
            sqlx::query(
                "insert into integration.discord_guild_roles \
                 (guild_id, role_id, name, managed, position) values ($1, $2, $3, $4, $5)",
            )
            .bind(&g.guild_id)
            .bind(&r.id)
            .bind(&r.name)
            .bind(r.managed)
            .bind(r.position)
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
        }
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

async fn replace_map(
    tx: &mut Transaction<'_, Postgres>,
    table: &str,
    id_col: &str,
    config_id: &str,
    entries: &[DiscordMapEntry],
) -> Result<(), ApiError> {
    sqlx::query(&format!(
        "delete from integration.{table} where config_id = $1"
    ))
    .bind(config_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    for e in entries {
        if e.name.trim().is_empty() || e.id.trim().is_empty() {
            continue;
        }
        sqlx::query(&format!(
            "insert into integration.{table} (config_id, name, {id_col}) values ($1, $2, $3) \
             on conflict (config_id, name) do update set {id_col} = excluded.{id_col}"
        ))
        .bind(config_id)
        .bind(e.name.trim())
        .bind(e.id.trim())
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    #[sqlx::test]
    async fn event_thread_template_get_returns_the_seeded_default(pool: PgPool) {
        let body = get_event_thread_template(&pool).await.unwrap();
        assert!(
            body.contains("{{title}}"),
            "seeded default carries the placeholders"
        );
    }

    #[sqlx::test]
    async fn event_thread_template_set_then_get_round_trips(pool: PgPool) {
        let updated = set_event_thread_template(&pool, "Custom: {{title}}")
            .await
            .unwrap();
        assert_eq!(updated, "Custom: {{title}}");
        assert_eq!(
            get_event_thread_template(&pool).await.unwrap(),
            "Custom: {{title}}"
        );
    }
}
