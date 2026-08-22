-- ACE support: controllers request live ACE-team coverage; the ACE team works a shared queue
-- (open → claimed → completed/cancelled) and keeps a roster. Discord posting is deferred (no bot
-- yet) — the `discord_message_id` column is reserved for it.

create table if not exists ace.requests (
    id             text primary key default gen_random_uuid()::text,
    requested_by   text not null references identity.users(id) on delete cascade,
    artcc_id       text,                       -- ARTCC the request is for (NULL = unspecified)
    position       text,                       -- position/facility support is wanted for (free text)
    requested_for  timestamptz,                -- desired coverage time, if supplied
    details        text not null default '',
    status         text not null default 'open'
        check (status in ('open', 'claimed', 'completed', 'cancelled')),
    claimed_by     text references identity.users(id) on delete set null,
    claimed_at     timestamptz,
    decided_by     text references identity.users(id) on delete set null,
    decided_at     timestamptz,
    discord_message_id text,                   -- reserved for the deferred Discord embed
    created_at     timestamptz not null default now(),
    updated_at     timestamptz not null default now()
);

create index if not exists idx_ace_requests_status on ace.requests(status, created_at desc);

create trigger trg_ace_requests_updated_at
before update on ace.requests
for each row execute function platform.touch_updated_at();

-- The ACE team roster (replaces the hardcoded display).
create table if not exists ace.team_members (
    id         text primary key default gen_random_uuid()::text,
    user_id    text not null unique references identity.users(id) on delete cascade,
    role       text,                           -- display role within the team (lead/member)
    artcc_id   text,                           -- home ARTCC, if presented by facility
    active     boolean not null default true,  -- soft-hide instead of delete
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create trigger trg_ace_team_members_updated_at
before update on ace.team_members
for each row execute function platform.touch_updated_at();

-- Permissions (catalog draft → seeded here) + role grants: any controller can request and see the
-- roster; the ACE team works the queue and manages the roster.
insert into access.permissions (name, description) values
    ('ace.requests.read',   'View the ACE support request queue'),
    ('ace.requests.create', 'Open an ACE support request'),
    ('ace.requests.claim',  'Claim an open ACE support request'),
    ('ace.requests.decide', 'Complete or cancel an ACE support request'),
    ('ace.team.read',       'View the ACE team roster'),
    ('ace.team.update',     'Manage the ACE team roster')
on conflict (name) do nothing;

insert into access.role_permissions (role_name, permission_name) values
    ('USER', 'ace.requests.create'),
    ('USER', 'ace.team.read'),
    ('ACE',  'ace.requests.read'),
    ('ACE',  'ace.requests.create'),
    ('ACE',  'ace.requests.claim'),
    ('ACE',  'ace.requests.decide'),
    ('ACE',  'ace.team.read'),
    ('ACE',  'ace.team.update')
on conflict do nothing;
