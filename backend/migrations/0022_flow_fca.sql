-- @formatter:off
-- Flow Constrained Areas (FCAs): a controller-drawn polyline across airspace that the
-- metering engine (added in later passes) sequences crossing traffic against. Ported
-- from vatflow. Shared + server-side: one FCA set, visible to every controller.

create table if not exists flow.fca (
    id text primary key default gen_random_uuid()::text,
    name text not null default '',
    color text not null default '#f59e0b',
    artcc text not null default '',                 -- owning facility (sidebar filter)
    -- geometry: array of [lat, lon] vertices — an open zero-width polyline (>= 2 pts)
    points jsonb not null default '[]'::jsonb,
    -- membership filters
    dests text[] not null default '{}',
    origins text[] not null default '{}',
    fixes text[] not null default '{}',
    scope text[] not null default '{}',             -- ARTCCs the FCA applies in
    min_fl int,                                     -- FL band; null = SFC / UNL
    max_fl int,
    dir text not null default 'any',                -- any | N | S | E | W
    -- metering config
    mode text not null default 'rate' check (mode in ('rate', 'mit')),
    rate int not null default 30 check (rate between 0 and 240),
    mit int not null default 15 check (mit between 0 and 200),
    enabled boolean not null default true,
    created_by text references identity.users(id) on delete set null,
    updated_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists idx_flow_fca_artcc on flow.fca(artcc);

create trigger trg_flow_fca_updated_at
before update on flow.fca
for each row execute function platform.touch_updated_at();

insert into access.permissions (name, description) values
    ('flow.fca.read', 'View flow constrained areas'),
    ('flow.fca.update', 'Create and edit flow constrained areas'),
    ('flow.fca.delete', 'Delete flow constrained areas')
on conflict (name) do nothing;
