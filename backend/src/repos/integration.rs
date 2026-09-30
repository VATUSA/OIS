//! Discord integration persistence: the outbound-job queue (backend enqueues in its own tx; the bot
//! leases → acks) and the guild config (logical name → snowflake maps). The bot owns no data.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    errors::ApiError,
    handlers::events::normalize_facility,
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
/// or `failed` once `MAX_ATTEMPTS` is reached.
///
/// Both branches require the job to still be `in_progress` (#446 review). Before the reaper existed that
/// was unnecessary — one leaser, one acker, and nothing else ever moved a row out of `in_progress`. The
/// reaper breaks that on purpose, so an ack can now arrive from a worker that no longer owns the job.
///
/// What this guard closes is the case that loses a delivery: the job is back in `pending` (or already
/// terminal) and the reaped worker acks anyway. Unfenced, a stale *success* marked that pending row
/// `succeeded` — never delivered, never retried, the exact silent loss #446 exists to stop.
///
/// The status alone could not close the other half: once a successor has re-leased the job the row is
/// `in_progress` again, and a status check cannot tell the two workers apart. `attempt` fences that
/// (VATUSA/OIS#472) — `lease_jobs` increments `attempt_count` and returns it, so a worker echoes back
/// the number identifying its own lease and a predecessor's late ack no longer matches.
///
/// `attempt` is `None` for a worker that doesn't send one, which falls back to the status-only fence.
/// That is deliberate: a required value would reject every ack from an old bot during the window
/// between deploying the two halves.
///
/// Returns false when nothing was updated: the id doesn't exist, or the ack is stale. The caller logs
/// that rather than discarding it, because a stale ack means a worker ran past its lease.
pub async fn ack_job(
    pool: &PgPool,
    id: &str,
    success: bool,
    result: Option<&Value>,
    error: Option<&str>,
    attempt: Option<i32>,
) -> Result<bool, ApiError> {
    let res = if success {
        let result =
            result.map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".to_string()));
        sqlx::query(
            "update integration.outbound_jobs \
             set status = 'succeeded', result = $2::jsonb, error = null \
             where id = $1 and status = 'in_progress' \
               and ($3::int is null or attempt_count = $3)",
        )
        .bind(id)
        .bind(result)
        .bind(attempt)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?
    } else {
        sqlx::query(
            "update integration.outbound_jobs \
             set status = case when attempt_count >= $2 then 'failed' else 'pending' end, \
                 next_attempt_at = now() + (interval '30 seconds' * least(attempt_count, 10)), \
                 error = $3 \
             where id = $1 and status = 'in_progress' \
               and ($4::int is null or attempt_count = $4)",
        )
        .bind(id)
        .bind(MAX_ATTEMPTS)
        .bind(error)
        .bind(attempt)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?
    };
    Ok(res.rows_affected() > 0)
}

/// Return jobs stranded `in_progress` to the queue (#446).
///
/// `lease_jobs` marks a job `in_progress` and stamps `last_attempt_at`; only the bot acking it moves
/// it on. So a worker that dies between leasing and acking — a redeploy, a crash, a dropped
/// connection — leaves the job there permanently. It is never retried and never delivered: an event
/// post or TMI that silently does not happen, with no error anywhere, because nothing ever failed.
///
/// A stranded job is treated exactly as a failed ack treats one — same backoff, same
/// `MAX_ATTEMPTS` terminal — so there is one retry policy rather than two that can drift. The only
/// difference is the error text, and it is written with `coalesce` so a real failure reason already
/// recorded is not overwritten by this generic one.
///
/// `last_attempt_at is null` is included for completeness: the column is nullable, and a NULL never
/// satisfies `<`, so such a row would be stranded permanently — the very bug this fixes. `lease_jobs` is
/// the only writer of `in_progress` and always stamps it, so this is unreachable today; that is an
/// invariant held by one call site rather than by the schema, and the guarantee here costs nothing to
/// make unconditional (#446 review).
///
/// **Delivery is at-least-once, deliberately.** If the worker posted to Discord and died before
/// acking, re-leasing posts again. Detecting that would need the bot to record the message before
/// sending it, which is a larger change than this one; a duplicate post is recoverable by hand
/// whereas a silently undelivered TMI is not, so the duplicate is the better failure to have.
pub async fn reap_stranded_jobs(
    pool: &PgPool,
    stranded_before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    sqlx::query(
        "update integration.outbound_jobs \
         set status = case when attempt_count >= $2 then 'failed' else 'pending' end, \
             next_attempt_at = now() + (interval '30 seconds' * least(attempt_count, 10)), \
             error = coalesce(error, 'lease expired: the worker never acked') \
         where status = 'in_progress' \
           and (last_attempt_at < $1 or last_attempt_at is null)",
    )
    .bind(stranded_before)
    .bind(MAX_ATTEMPTS)
    .execute(pool)
    .await
    .map(|r| r.rows_affected())
    .map_err(|_| ApiError::Internal)
}

/// Shared by `channel_id`/`role_id`: resolve `name` to its Discord snowflake in `table` (`id_col`
/// its snowflake column), preferring a guild whose config lists `facility` among its ARTCCs
/// (`integration.discord_config_facilities`, #194) over one that doesn't, falling back to
/// whichever guild is first in the configured (request-array) order when no guild matches (or
/// none is given) — same as before facility-scoping existed. `table`/`id_col` are always one of
/// the two hardcoded literals below, never caller/user input, so building the query with `format!`
/// carries no injection risk.
async fn resolve_scoped_id(
    pool: &PgPool,
    table: &str,
    id_col: &str,
    name: &str,
    facility: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let query = format!(
        "select t.{id_col} from integration.{table} t \
         join integration.discord_configs c on c.id = t.config_id \
         left join integration.discord_config_facilities f \
           on f.config_id = c.id and f.artcc_id = $2 \
         where t.name = $1 \
         order by (f.artcc_id is not null) desc, c.sort_order \
         limit 1"
    );
    sqlx::query_scalar::<_, String>(&query)
        .bind(name)
        .bind(facility)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Resolve a logical channel name to its Discord snowflake (None if unmapped / no config). Callers
/// skip enqueuing a Discord job when there's nowhere to post. See `resolve_scoped_id` for the
/// facility-preference/fallback behavior.
pub async fn channel_id(
    pool: &PgPool,
    name: &str,
    facility: Option<&str>,
) -> Result<Option<String>, ApiError> {
    resolve_scoped_id(pool, "discord_channels", "channel_id", name, facility).await
}

/// Resolve a logical role name to its Discord snowflake. See `resolve_scoped_id` for the
/// facility-preference/fallback behavior.
pub async fn role_id(
    pool: &PgPool,
    name: &str,
    facility: Option<&str>,
) -> Result<Option<String>, ApiError> {
    resolve_scoped_id(pool, "discord_roles", "role_id", name, facility).await
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
/// The channel a TMI's publish post was actually sent to, if it was ever enqueued (#436 review).
///
/// A cancellation has to land beside the row it corrects, and re-deriving the channel does not get
/// there: `publish_tmi` resolves it unscoped while `activate_package` resolves it with the event's
/// facility, so `resolve_scoped_id` can legitimately answer with two different guilds for the same
/// logical name. Reading it back off the publish job is exact and needs no decision about scoping.
///
/// Deliberately not filtered on `status`: a TMI cancelled moments after publishing has a job that is
/// still `pending`, and that job's channel is still the right answer. `(subject_type, subject_id)` is
/// indexed (`0048_integration_discord.sql:28`).
pub async fn published_channel_for_tmi(
    pool: &PgPool,
    tmi_id: &str,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, Option<String>>(
        "select payload->>'channel_id' from integration.outbound_jobs \
         where subject_type = 'tmi' and subject_id = $1 and job_type = 'tmi_publish' \
         order by created_at desc limit 1",
    )
    .bind(tmi_id)
    .fetch_optional(pool)
    .await
    .map(Option::flatten)
    .map_err(|_| ApiError::Internal)
}

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

/// The ARTCCs a guild's config serves (#194) — see `channel_id`/`role_id`.
async fn facility_entries(pool: &PgPool, config_id: &str) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select artcc_id from integration.discord_config_facilities \
         where config_id = $1 order by artcc_id",
    )
    .bind(config_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The full Discord config: every configured guild + its maps, plus the bot's guild snapshot for
/// the editor's dropdowns. Empty `guilds` when nothing's been configured yet.
pub async fn get_config(pool: &PgPool) -> Result<DiscordConfigBody, ApiError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "select id, name, guild_id from integration.discord_configs order by sort_order",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let mut guilds = Vec::with_capacity(rows.len());
    for (id, name, guild_id) in rows {
        guilds.push(DiscordGuildConfigBody {
            channels: map_entries(pool, "discord_channels", "channel_id", &id).await?,
            roles: map_entries(pool, "discord_roles", "role_id", &id).await?,
            facilities: facility_entries(pool, &id).await?,
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
    for (i, g) in req.guilds.iter().enumerate() {
        if g.guild_id.trim().is_empty() {
            continue;
        }
        // sort_order is the request array's position, not a re-densified counter — a skipped
        // (empty guild_id) entry leaves a gap, which is harmless for ordering.
        let config_id = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id, sort_order) \
             values ($1, $2, $3) returning id",
        )
        .bind(g.name.trim())
        .bind(g.guild_id.trim())
        .bind(i as i32)
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
        for artcc in &g.facilities {
            // Same validation as everywhere else an ARTCC/facility id is accepted (events.rs) —
            // looser validation here would silently store junk that can never match a real
            // event/request facility, defeating the facility-preference lookup with no error.
            let Some(artcc) = normalize_facility(artcc) else {
                continue;
            };
            sqlx::query(
                "insert into integration.discord_config_facilities (config_id, artcc_id) \
                 values ($1, $2) on conflict do nothing",
            )
            .bind(&config_id)
            .bind(&artcc)
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
        }
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}

// --- event-thread message template (singleton row; see migration 0065) ---------------------------

/// Fallback if the seeded 'default' row is ever missing (manual DB fix, botched rollback) — same
/// text migration 0065 seeds. Degrading to this keeps event-thread creation working rather than
/// 500ing every `publish_event_discord` call on what would otherwise be a single point of failure.
const FALLBACK_EVENT_THREAD_TEMPLATE: &str = "**{{title}} | Planning Thread**\n\
    {{title}} is on {{date_line}}\n\n\
    Review the following for your facility:\n\
    - TMU/TMI package\n\
    - Staffing\n\
    - Configs and AAR\n\n\
    {{facility_lines}}\n\
    Attempt to coordinate as many plans (initiatives, reroutes, etc.) in a timely manner, and fill \
    out all appropriate areas of the staffing data.\n\
    ───────────────────────────\n\
    {{ntmo_ping}} please react with your availability to NOM for this event. {{dcc_ping}} please \
    react with your availability to shadow this event.\n\n\
    🟢 = Available\n🟡 = Partially available/unsure\n🔴 = Unavailable\n\
    ───────────────────────────";

/// The configured event-thread message body (placeholders substituted by the bot at render time).
pub async fn get_event_thread_template(pool: &PgPool) -> Result<String, ApiError> {
    let row = sqlx::query_scalar::<_, String>(
        "select body from integration.event_thread_template where id = 'default'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(row.unwrap_or_else(|| {
        tracing::warn!("event_thread_template 'default' row missing — using compiled-in fallback");
        FALLBACK_EVENT_THREAD_TEMPLATE.to_string()
    }))
}

pub async fn set_event_thread_template(pool: &PgPool, body: &str) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into integration.event_thread_template (id, body) values ('default', $1) \
         on conflict (id) do update set body = excluded.body \
         returning body",
    )
    .bind(body)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
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
    use crate::models::DiscordGuildConfigInput;

    /// A job sitting `in_progress`, last touched `mins_ago`, with `attempt_count` attempts behind it.
    async fn stranded_job(pool: &PgPool, mins_ago: i64, attempt_count: i32) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into integration.outbound_jobs \
             (job_type, status, attempt_count, last_attempt_at) \
             values ('tmi_publish', 'in_progress', $1, now() - make_interval(mins => $2)) \
             returning id",
        )
        .bind(attempt_count)
        .bind(mins_ago as i32)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn status_of(pool: &PgPool, id: &str) -> String {
        sqlx::query_scalar::<_, String>(
            "select status from integration.outbound_jobs where id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// #446: only an ack moves a job out of `in_progress`, so a worker that died mid-job left it
    /// there forever — never retried, never delivered, and with no error, because nothing failed.
    #[sqlx::test]
    async fn a_job_whose_worker_died_is_returned_to_the_queue(pool: PgPool) {
        let stranded = stranded_job(&pool, 10, 1).await;
        let working = stranded_job(&pool, 1, 1).await;
        let cutoff = Utc::now() - chrono::Duration::minutes(5);

        assert_eq!(reap_stranded_jobs(&pool, cutoff).await.unwrap(), 1);

        assert_eq!(status_of(&pool, &stranded).await, "pending");
        assert_eq!(
            status_of(&pool, &working).await,
            "in_progress",
            "a job still inside its lease is being worked on, not abandoned"
        );
    }

    /// The recovery reuses the failed-ack transition, so a stranded job that has already exhausted
    /// its attempts becomes terminal rather than looping forever.
    #[sqlx::test]
    async fn a_stranded_job_out_of_attempts_becomes_failed(pool: PgPool) {
        let exhausted = stranded_job(&pool, 10, MAX_ATTEMPTS).await;

        reap_stranded_jobs(&pool, Utc::now() - chrono::Duration::minutes(5))
            .await
            .unwrap();

        assert_eq!(status_of(&pool, &exhausted).await, "failed");
    }

    /// It must only ever touch `in_progress`. A pending job is waiting its turn and a succeeded one
    /// is done; re-queueing either would deliver something twice for no reason.
    #[sqlx::test]
    async fn no_other_status_is_disturbed(pool: PgPool) {
        for status in ["pending", "succeeded", "failed"] {
            let id = sqlx::query_scalar::<_, String>(
                "insert into integration.outbound_jobs (job_type, status, last_attempt_at) \
                 values ('tmi_publish', $1, now() - interval '1 hour') returning id",
            )
            .bind(status)
            .fetch_one(&pool)
            .await
            .unwrap();

            reap_stranded_jobs(&pool, Utc::now() - chrono::Duration::minutes(5))
                .await
                .unwrap();

            assert_eq!(
                status_of(&pool, &id).await,
                status,
                "{status} was disturbed"
            );
        }
    }

    /// A real failure reason already on the row is the useful one; the generic lease-expiry text
    /// must not overwrite it.
    #[sqlx::test]
    async fn an_existing_error_is_not_overwritten(pool: PgPool) {
        let id = stranded_job(&pool, 10, 1).await;
        sqlx::query(
            "update integration.outbound_jobs set error = 'channel not found' where id = $1",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();

        reap_stranded_jobs(&pool, Utc::now() - chrono::Duration::minutes(5))
            .await
            .unwrap();

        let error: Option<String> =
            sqlx::query_scalar("select error from integration.outbound_jobs where id = $1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(error.as_deref(), Some("channel not found"));
    }

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

    /// If the seeded 'default' row is ever missing, get_event_thread_template must degrade to the
    /// compiled-in fallback rather than error — a single point of failure that would otherwise block
    /// every `publish_event_discord` call.
    #[sqlx::test]
    async fn missing_default_row_degrades_to_the_compiled_in_fallback(pool: PgPool) {
        sqlx::query("delete from integration.event_thread_template where id = 'default'")
            .execute(&pool)
            .await
            .unwrap();
        let body = get_event_thread_template(&pool).await.unwrap();
        assert_eq!(body, FALLBACK_EVENT_THREAD_TEMPLATE);
    }

    /// `sort_order` is explicit so two guilds seeded in one test have a deterministic
    /// fallback-tiebreak order (#203 — `created_at` no longer participates in that tiebreak).
    async fn seed_guild(
        pool: &PgPool,
        guild_name: &str,
        channel_name: &str,
        sort_order: i32,
    ) -> String {
        let config_id = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id, sort_order) \
             values ($1, $2, $3) returning id",
        )
        .bind(guild_name)
        .bind(format!("{guild_name}-snowflake"))
        .bind(sort_order)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into integration.discord_channels (config_id, name, channel_id) \
             values ($1, $2, $3)",
        )
        .bind(&config_id)
        .bind(channel_name)
        .bind(format!("{config_id}-channel"))
        .execute(pool)
        .await
        .unwrap();
        config_id
    }

    /// Reproduces #194: two guilds configure the same logical channel name. Before the fix, the
    /// second (later-created) guild's mapping was always unreachable, regardless of facility.
    #[sqlx::test]
    async fn second_guild_with_same_name_routes_by_facility(pool: PgPool) {
        let first = seed_guild(&pool, "DCC", "aceteam-requests", 0).await;
        let second = seed_guild(&pool, "VATUSA", "aceteam-requests", 1).await;
        sqlx::query(
            "insert into integration.discord_config_facilities (config_id, artcc_id) \
             values ($1, 'ZDC')",
        )
        .bind(&second)
        .execute(&pool)
        .await
        .unwrap();

        // Facility-scoped lookup: the second (facility-matched) guild wins, not the first-created one.
        let scoped = channel_id(&pool, "aceteam-requests", Some("ZDC"))
            .await
            .unwrap();
        assert_eq!(scoped, Some(format!("{second}-channel")));

        // No facility given, or a facility no guild claims: falls back to the first-created guild,
        // exactly like before facility-scoping existed.
        let unscoped = channel_id(&pool, "aceteam-requests", None).await.unwrap();
        assert_eq!(unscoped, Some(format!("{first}-channel")));
        let unmatched = channel_id(&pool, "aceteam-requests", Some("ZAB"))
            .await
            .unwrap();
        assert_eq!(unmatched, Some(format!("{first}-channel")));
    }

    /// `role_id` shares `resolve_scoped_id` with `channel_id`, but is invoked with a distinct
    /// table/column pair ("discord_roles"/"role_id") — this proves that wiring is correct on its
    /// own, not just the shared query logic already covered above.
    #[sqlx::test]
    async fn second_guild_with_same_role_name_routes_by_facility(pool: PgPool) {
        let first = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id, sort_order) \
             values ('DCC', 'dcc-snowflake', 0) returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let second = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id, sort_order) \
             values ('VATUSA', 'vatusa-snowflake', 1) returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        for (config_id, role_snowflake) in [(&first, "first-role"), (&second, "second-role")] {
            sqlx::query(
                "insert into integration.discord_roles (config_id, name, role_id) \
                 values ($1, 'ntmo', $2)",
            )
            .bind(config_id)
            .bind(role_snowflake)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "insert into integration.discord_config_facilities (config_id, artcc_id) \
             values ($1, 'ZDC')",
        )
        .bind(&second)
        .execute(&pool)
        .await
        .unwrap();

        let scoped = role_id(&pool, "ntmo", Some("ZDC")).await.unwrap();
        assert_eq!(scoped, Some("second-role".to_string()));
        let unscoped = role_id(&pool, "ntmo", None).await.unwrap();
        assert_eq!(unscoped, Some("first-role".to_string()));
    }

    /// Reproduces #203: `upsert_config`'s full delete+reinsert transaction used to rely on
    /// `created_at`'s `default now()`, which is the *transaction's* start time in Postgres — every
    /// guild inserted in the loop got an identical timestamp, so the no-facility-match fallback
    /// (`resolve_scoped_id`'s `order by ..., c.sort_order`) depended on Postgres's unspecified
    /// same-value row order. Asserts the real `upsert_config` path gives each guild a distinct,
    /// request-order `sort_order`, and that the fallback deterministically prefers the first one.
    #[sqlx::test]
    async fn upsert_config_gives_each_guild_a_distinct_sort_order(pool: PgPool) {
        let req = UpsertDiscordConfigRequest {
            guilds: vec![
                DiscordGuildConfigInput {
                    name: "DCC".to_string(),
                    guild_id: "dcc-snowflake".to_string(),
                    channels: vec![DiscordMapEntry {
                        name: "ops".to_string(),
                        id: "dcc-ops-channel".to_string(),
                    }],
                    roles: vec![],
                    facilities: vec![],
                },
                DiscordGuildConfigInput {
                    name: "VATUSA".to_string(),
                    guild_id: "vatusa-snowflake".to_string(),
                    channels: vec![DiscordMapEntry {
                        name: "ops".to_string(),
                        id: "vatusa-ops-channel".to_string(),
                    }],
                    roles: vec![],
                    facilities: vec![],
                },
            ],
        };
        upsert_config(&pool, &req).await.unwrap();

        let sort_orders: Vec<i32> = sqlx::query_scalar(
            "select sort_order from integration.discord_configs order by sort_order",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            sort_orders,
            vec![0, 1],
            "each guild should get a distinct sort_order matching its request-array position"
        );

        // No facility given, and neither guild claims one: the fallback deterministically returns
        // the first-configured guild's mapping, every time — never a coin flip on row order.
        for _ in 0..5 {
            assert_eq!(
                channel_id(&pool, "ops", None).await.unwrap(),
                Some("dcc-ops-channel".to_string())
            );
        }
    }

    /// The lease fence, as far as a status check reaches (#446 review).
    ///
    /// This closes the case that loses a delivery outright: the reaper has returned the job to
    /// `pending` and the reaped worker's ack then arrives. Unfenced, a stale *success* marked that
    /// pending row `succeeded` — so the job was never delivered by anyone and nothing ever retried it,
    /// which is precisely the silent loss #446 exists to stop. A stale *failure* burned an attempt and
    /// pushed the backoff out for a job nobody was working.
    ///
    /// It covers only the status fence — these acks carry no lease token, which is also the
    /// old-bot path. The re-leased case is `a_predecessors_ack_cannot_touch_a_re_leased_job` below.
    #[sqlx::test]
    async fn an_ack_from_a_reaped_worker_is_refused_while_the_job_waits(pool: PgPool) {
        let id = stranded_job(&pool, 10, 1).await;
        assert_eq!(
            reap_stranded_jobs(&pool, Utc::now() - chrono::Duration::minutes(5))
                .await
                .unwrap(),
            1
        );
        assert_eq!(status_of(&pool, &id).await, "pending");

        assert!(
            !ack_job(&pool, &id, true, None, None, None).await.unwrap(),
            "a stale success must not apply; the bool is what lets the handler log it"
        );
        assert_eq!(
            status_of(&pool, &id).await,
            "pending",
            "the job must still be waiting to be re-leased, not marked delivered"
        );

        assert!(
            !ack_job(&pool, &id, false, None, Some("stale"), None)
                .await
                .unwrap()
        );
        assert_eq!(status_of(&pool, &id).await, "pending");

        // The successor's ack, once it holds the job, still works — the fence blocks the stale one only.
        sqlx::query("update integration.outbound_jobs set next_attempt_at = now() where id = $1")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            lease_jobs(&pool, 10)
                .await
                .unwrap()
                .iter()
                .any(|j| j.id == id)
        );
        assert!(ack_job(&pool, &id, true, None, None, None).await.unwrap());
        assert_eq!(status_of(&pool, &id).await, "succeeded");
    }

    /// The half a status fence cannot reach (VATUSA/OIS#472): once a successor has re-leased the job
    /// the row is `in_progress` again, so only the lease token tells the two workers apart.
    ///
    /// Driven the way it actually happens — lease, reap, re-lease, then both acks arrive.
    #[sqlx::test]
    async fn a_predecessors_ack_cannot_touch_a_re_leased_job(pool: PgPool) {
        let id = stranded_job(&pool, 10, 1).await;
        // Worker A holds attempt 1. The reaper returns the job, and worker B takes it as attempt 2.
        reap_stranded_jobs(&pool, Utc::now() - chrono::Duration::minutes(5))
            .await
            .unwrap();
        // The reaper backs the job off; bring it forward so the successor can take it now.
        sqlx::query("update integration.outbound_jobs set next_attempt_at = now() where id = $1")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        let b = lease_jobs(&pool, 10)
            .await
            .unwrap()
            .into_iter()
            .find(|j| j.id == id)
            .expect("the successor leases it");
        assert_eq!(b.attempt_count, 2, "the successor holds a later lease");
        assert_eq!(status_of(&pool, &id).await, "in_progress");

        // A's success arrives late. Unfenced this marked the job delivered while B was mid-flight,
        // and B's own ack — carrying the real message id — was then refused.
        assert!(
            !ack_job(&pool, &id, true, None, None, Some(1))
                .await
                .unwrap(),
            "a predecessor's success must not apply to its successor's lease"
        );
        assert_eq!(status_of(&pool, &id).await, "in_progress");

        // A's failure is refused too: unfenced it returned a job B was holding to `pending`, so a
        // third worker could take it while B was still running.
        assert!(
            !ack_job(&pool, &id, false, None, Some("stale"), Some(1))
                .await
                .unwrap()
        );
        assert_eq!(status_of(&pool, &id).await, "in_progress");

        // B's ack still applies, and its result is the one recorded.
        let result = serde_json::json!({"message_id": "123"});
        assert!(
            ack_job(&pool, &id, true, Some(&result), None, Some(2))
                .await
                .unwrap()
        );
        assert_eq!(status_of(&pool, &id).await, "succeeded");
        let stored: Option<Value> =
            sqlx::query_scalar("select result from integration.outbound_jobs where id = $1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            stored,
            Some(result),
            "the successor's result is the one kept"
        );
    }

    /// An ack with no lease token must behave exactly as before, or deploying the backend ahead of
    /// the bot would reject every ack and turn a narrow race into total delivery failure.
    #[sqlx::test]
    async fn an_ack_without_a_lease_token_still_applies(pool: PgPool) {
        let id = stranded_job(&pool, 10, 1).await;
        assert_eq!(status_of(&pool, &id).await, "in_progress");
        assert!(
            ack_job(&pool, &id, true, None, None, None).await.unwrap(),
            "an old bot sends no attempt and must keep working"
        );
        assert_eq!(status_of(&pool, &id).await, "succeeded");
    }

    /// A terminal job is not ackable either, which is the other half of what the fence buys: a late ack
    /// cannot resurrect a job that already failed out of its attempts.
    #[sqlx::test]
    async fn an_ack_against_a_terminal_job_is_refused(pool: PgPool) {
        let id = stranded_job(&pool, 10, MAX_ATTEMPTS).await;
        reap_stranded_jobs(&pool, Utc::now() - chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert_eq!(status_of(&pool, &id).await, "failed");

        assert!(!ack_job(&pool, &id, true, None, None, None).await.unwrap());
        assert_eq!(status_of(&pool, &id).await, "failed");
    }

    /// An `in_progress` row whose `last_attempt_at` is NULL would never satisfy `<`, so it would be
    /// stranded permanently — the bug this reaper exists to fix (#446 review). Unreachable through
    /// `lease_jobs`, which always stamps it; pinned because that is an invariant of one call site rather
    /// than of the schema, and the column is nullable.
    #[sqlx::test]
    async fn an_in_progress_job_with_no_lease_stamp_is_still_reaped(pool: PgPool) {
        let id = sqlx::query_scalar::<_, String>(
            "insert into integration.outbound_jobs (job_type, status, attempt_count) \
             values ('tmi_publish', 'in_progress', 1) returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(
            reap_stranded_jobs(&pool, Utc::now() - chrono::Duration::minutes(5))
                .await
                .unwrap(),
            1
        );
        assert_eq!(status_of(&pool, &id).await, "pending");
    }
}
