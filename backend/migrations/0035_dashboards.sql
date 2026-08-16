-- Multiple named dashboards per user (replaces the single user_preferences "dashboard" blob),
-- optionally grouped into collections and shareable by an unguessable slug. `data` holds the
-- client-owned DashboardState (widgets + grid layout), opaque to the backend.
create table identity.dashboard_collections (
    id         text primary key default gen_random_uuid()::text,
    owner_id   text not null references identity.users(id) on delete cascade,
    name       text not null,
    created_at timestamptz not null default now()
);

create table identity.dashboards (
    id            text primary key default gen_random_uuid()::text,
    owner_id      text not null references identity.users(id) on delete cascade,
    collection_id text references identity.dashboard_collections(id) on delete set null,
    name          text not null,
    data          jsonb not null default '{}'::jsonb,
    -- null = private; a random slug = shareable at /ops/my/shared/<slug> to any signed-in user.
    share_slug    text unique,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);

create index idx_dashboards_owner on identity.dashboards(owner_id);
create index idx_dashboard_collections_owner on identity.dashboard_collections(owner_id);
