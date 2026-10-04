-- @formatter:off
-- VATUSA/OIS#656: the job queue's consumer is now the calling service account's key, and the Discord
-- bot leases as `discord` (repos::integration::DISCORD_CONSUMER). A bot account keyed anything else gets
-- 403 on every lease, and its pending jobs can age out, so a forgotten manual rename would silently
-- lose Discord notifications. Rename it here instead.
--
-- Guarded, because a wrong guess would hand the bot's jobs to another account: only when exactly one
-- active service account holds a current BOT grant, and no account is keyed `discord` yet. Anything
-- else (no bot, two candidates, already renamed) is left alone, and docs/deploy.md covers it.
--
-- `id in (...)`, not `id = (...)`: with two candidates a scalar subquery raises "more than one row" if
-- Postgres evaluates it before the count guard. It needn't evaluate the conditions in written order,
-- and a failed migration stops the backend starting, so don't depend on that order.
with bots as (
    select distinct sar.service_account_id as id
    from access.service_account_roles sar
    join access.service_accounts sa on sa.id = sar.service_account_id
    where sar.role_name = 'BOT'
      and sa.status = 'active'
      and sar.starts_at <= now()
      and (sar.ends_at is null or sar.ends_at > now())
)
update access.service_accounts
set key = 'discord', updated_at = now()
where id in (select id from bots)
  and (select count(*) from bots) = 1
  and not exists (select 1 from access.service_accounts where key = 'discord');
