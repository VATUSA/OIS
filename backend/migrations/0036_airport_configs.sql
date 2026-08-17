-- @formatter:off
-- Reusable per-airport runway configurations: a named config with a favored-wind rule and an
-- AAR/ADR. Used to predict an event's arrival rate from the forecast wind (with a manual override
-- in the event manager). Facility-scoped: managed by staff holding events.config.update for the
-- airport's owning ARTCC, or nationally.

create table if not exists flow.airport_config (
    id text primary key default gen_random_uuid()::text,
    icao text not null,
    name text not null,                          -- e.g. "South Flow"
    aar int not null default 0 check (aar between 0 and 200),
    adr int not null default 0 check (adr between 0 and 200),
    landing_runways text[] not null default '{}',
    -- Favored-wind rule: this config applies when the surface wind direction is within
    -- [wind_from_deg, wind_to_deg] (inclusive; wrap-around allowed, e.g. 300..60). Ignored when
    -- calm_default is true.
    wind_from_deg int not null default 0 check (wind_from_deg between 0 and 360),
    wind_to_deg int not null default 360 check (wind_to_deg between 0 and 360),
    -- The config used when the wind is light/variable or nothing matches. At most one per airport.
    calm_default boolean not null default false,
    artcc text not null default '',              -- owning ARTCC (scope audit / display)
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now()
);

create index if not exists idx_airport_config_icao on flow.airport_config(icao);
create unique index if not exists idx_airport_config_calm
    on flow.airport_config(icao) where calm_default;

create trigger trg_airport_config_updated_at
before update on flow.airport_config
for each row execute function platform.touch_updated_at();

-- Tie an event's per-airport rate to the chosen config, and record whether it's the weather
-- prediction or a manual override.
alter table events.airport_rate
    add column if not exists config_id text references flow.airport_config(id) on delete set null,
    add column if not exists source text not null default 'override'
        check (source in ('predicted', 'override'));

insert into access.permissions (name, description) values
    ('events.config.update', 'Manage an airport''s default runway configurations (facility-scoped)')
on conflict (name) do nothing;
