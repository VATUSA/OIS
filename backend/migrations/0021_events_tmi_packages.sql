-- @formatter:off
-- TMI packages: a named bundle of draft TMIs (programs / restrictions / ground stops)
-- planned for an event. Activating a package materializes live tmu.* rows.

create table if not exists events.tmi_package (
    id text primary key default gen_random_uuid()::text,
    event_id bigint not null references events.event(id) on delete cascade,
    name text not null default '',
    -- draft | activated
    status text not null default 'draft' check (status in ('draft', 'activated')),
    activated_at timestamptz,
    updated_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists idx_events_tmi_package_event on events.tmi_package(event_id);

create table if not exists events.tmi_package_item (
    id text primary key default gen_random_uuid()::text,
    package_id text not null references events.tmi_package(id) on delete cascade,
    -- program | restriction | ground_stop
    kind text not null check (kind in ('program', 'restriction', 'ground_stop')),
    payload jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now()
);

create index if not exists idx_events_tmi_package_item_pkg on events.tmi_package_item(package_id);

create trigger trg_events_tmi_package_updated_at
before update on events.tmi_package
for each row execute function platform.touch_updated_at();
