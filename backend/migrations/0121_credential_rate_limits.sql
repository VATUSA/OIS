-- Per-credential rate limits and usage (#611, deferred from #588).
--
-- `rate_limit_per_min`: an admin's override of RATE_LIMIT_CREDENTIAL_PER_MIN for one key or service
-- account; null is the deployment default. Read by the bearer lookup the auth middleware already runs,
-- so a change applies on the next request on every replica.
alter table access.api_keys
    add column if not exists rate_limit_per_min integer check (rate_limit_per_min > 0);
alter table access.service_accounts
    add column if not exists rate_limit_per_min integer check (rate_limit_per_min > 0);

-- Request volume per credential per hour. Each replica counts in memory and adds its counts here once a
-- minute (`jobs::spawn_credential_usage_flush`), so a row is the sum across replicas. Kept 7 days.
-- No foreign key: the id names a row in one of two tables, and usage of a deleted key ages out.
create table if not exists access.credential_usage (
    kind          text not null check (kind in ('api_key', 'service_account')),
    credential_id text not null,
    hour          timestamptz not null,
    requests      bigint not null default 0,
    refused       bigint not null default 0,
    primary key (kind, credential_id, hour)
);
