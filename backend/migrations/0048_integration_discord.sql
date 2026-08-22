-- Discord integration foundation: the outbound-job queue (backend enqueues side-effects in its own
-- tx; the bot leases → performs → acks), the Discord guild/channel/role/category config (features
-- reference channels/roles by logical name, never raw snowflakes), and the VATSIM↔Discord identity
-- map. The bot owns no data — everything it needs lives here behind the API.

-- --- outbound job queue -------------------------------------------------------------------------

create table if not exists integration.outbound_jobs (
    id              text primary key default gen_random_uuid()::text,
    job_type        text not null,               -- discriminator selecting the bot's handler
    payload         jsonb not null default '{}',  -- resolved channel/role ids + embed fields + subject ids
    subject_type    text,                         -- originating row kind (e.g. 'ace_request') for idempotency/audit
    subject_id      text,
    status          text not null default 'pending'
        check (status in ('pending', 'in_progress', 'succeeded', 'failed')),
    attempt_count   int  not null default 0,
    next_attempt_at timestamptz not null default now(),  -- eligible-at; pushed out for backoff
    last_attempt_at timestamptz,
    error           text,                         -- last failure reason
    result          jsonb,                        -- ids the bot returns on ack (e.g. message/thread id)
    created_at      timestamptz not null default now(),
    updated_at      timestamptz not null default now()
);

-- Due-pending jobs, oldest first (the lease query).
create index if not exists idx_outbound_jobs_due
    on integration.outbound_jobs (next_attempt_at) where status = 'pending';
create index if not exists idx_outbound_jobs_subject
    on integration.outbound_jobs (subject_type, subject_id);

create trigger trg_integration_outbound_jobs_updated_at
before update on integration.outbound_jobs
for each row execute function platform.touch_updated_at();

-- --- discord config (one guild for the first cut, but modeled for many) --------------------------

create table if not exists integration.discord_configs (
    id         text primary key default gen_random_uuid()::text,
    name       text not null,
    guild_id   text not null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);
create trigger trg_integration_discord_configs_updated_at
before update on integration.discord_configs
for each row execute function platform.touch_updated_at();

-- Logical name → snowflake maps, scoped to a config. Features resolve by logical name at enqueue time.
create table if not exists integration.discord_channels (
    id         text primary key default gen_random_uuid()::text,
    config_id  text not null references integration.discord_configs(id) on delete cascade,
    name       text not null,                    -- logical name, e.g. 'tmu', 'aceteam-requests'
    channel_id text not null,
    unique (config_id, name)
);
create table if not exists integration.discord_roles (
    id         text primary key default gen_random_uuid()::text,
    config_id  text not null references integration.discord_configs(id) on delete cascade,
    name       text not null,
    role_id    text not null,
    unique (config_id, name)
);
create table if not exists integration.discord_categories (
    id          text primary key default gen_random_uuid()::text,
    config_id   text not null references integration.discord_configs(id) on delete cascade,
    name        text not null,
    category_id text not null,
    unique (config_id, name)
);

-- --- VATSIM ↔ external identity map (Discord account linking) -------------------------------------

create table if not exists integration.external_sync_mappings (
    id          text primary key default gen_random_uuid()::text,
    system_code text not null,                   -- 'discord'
    entity_type text not null,                   -- 'user'
    local_id    text not null,                   -- OIS user id
    external_id text not null,                   -- Discord user id
    metadata    jsonb,                           -- e.g. discord username/avatar at link time
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now(),
    unique (system_code, entity_type, local_id)
);
create trigger trg_integration_external_sync_mappings_updated_at
before update on integration.external_sync_mappings
for each row execute function platform.touch_updated_at();

-- --- permissions ---------------------------------------------------------------------------------

insert into access.permissions (name, description) values
    ('discord.config.read',      'View the Discord guild/channel/role/category mapping'),
    ('discord.config.update',    'Edit the Discord mapping'),
    ('integration.jobs.update', 'Lease and acknowledge outbound integration jobs (the bot)')
on conflict (name) do nothing;

-- The bot's service account (role BOT) drains the queue and claims ACE requests on a user's behalf.
insert into access.role_permissions (role_name, permission_name) values
    ('BOT', 'integration.jobs.update'),
    ('BOT', 'ace.requests.claim')
on conflict do nothing;
