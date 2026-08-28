-- Snapshot of the Discord guilds the bot is in, plus their channels + roles. The bot pushes this
-- (on connect + on a manual refresh) so the admin config UI can offer dropdowns instead of hand-typed
-- snowflakes. This is the "what actually exists in the server" list — distinct from the logical-name →
-- snowflake MAPPINGS in integration.discord_channels/discord_roles, which the admin still curates.

create table if not exists integration.discord_guilds (
    guild_id  text primary key,
    name      text not null default '',
    synced_at timestamptz not null default now()
);

create table if not exists integration.discord_guild_channels (
    guild_id   text not null references integration.discord_guilds(guild_id) on delete cascade,
    channel_id text not null,
    name       text not null default '',
    kind       text not null default 'text',   -- text | voice | category | forum | announcement | …
    parent_id  text,                            -- category id, for grouping in the dropdown
    position   int  not null default 0,
    primary key (guild_id, channel_id)
);

create table if not exists integration.discord_guild_roles (
    guild_id text not null references integration.discord_guilds(guild_id) on delete cascade,
    role_id  text not null,
    name     text not null default '',
    color    bigint not null default 0,
    position int  not null default 0,
    managed  boolean not null default false,    -- bot/integration-managed roles (not assignable)
    primary key (guild_id, role_id)
);

-- The categories map is unused — drop it (the config UI no longer offers it).
drop table if exists integration.discord_categories;
