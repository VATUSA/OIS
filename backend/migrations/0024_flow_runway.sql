-- @formatter:off
-- Runway Balancer: shared, per-airport runway configuration for assigning live arrivals to
-- landing runways. Ported from vatflow's client-side tool, but stored server-side so every
-- controller shares one picture. The assignment itself is computed live off the feed.

create table if not exists flow.runway_config (
    icao text primary key,
    active_ends text[] not null default '{}',       -- selected landing-runway end ids
    star_rules jsonb not null default '{}'::jsonb,   -- { STAR_base: runway_end_id }
    overrides jsonb not null default '{}'::jsonb,    -- { callsign: runway_end_id }
    window_min int not null default 90 check (window_min between 30 and 240),
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now()
);

create trigger trg_flow_runway_config_updated_at
before update on flow.runway_config
for each row execute function platform.touch_updated_at();

-- Named, reusable runway configurations per airport (e.g. "West Ops").
create table if not exists flow.runway_saved_config (
    icao text not null,
    name text not null,
    payload jsonb not null default '{}'::jsonb,      -- { ends:[{id,active,hdg,len,pair}], rules:{} }
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (icao, name)
);

insert into access.permissions (name, description) values
    ('flow.runway.read', 'View the runway balancer'),
    ('flow.runway.update', 'Edit runway config, STAR rules, and assignments')
on conflict (name) do nothing;
