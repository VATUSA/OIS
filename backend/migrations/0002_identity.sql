-- Minimal identity for the auth vertical slice. Trimmed from osmium (no profiles,
-- flags, memberships yet) — rating lives on the user row for now; the fuller roster
-- model arrives with VATUSA sync.
create table if not exists identity.users
(
    id
    text
    primary
    key
    default
    gen_random_uuid
(
)::text,
    cid bigint unique,
    email citext unique,
    full_name text not null,
    display_name text not null,
    rating text,
    status text not null default 'ACTIVE' check
(
    status
    in
(
    'ACTIVE',
    'INACTIVE',
    'SUSPENDED'
)),
    created_at timestamptz not null default now
(
),
    updated_at timestamptz not null default now
(
)
    );

create index if not exists idx_identity_users_status on identity.users(status);

create table if not exists identity.sessions
(
    id
    text
    primary
    key
    default
    gen_random_uuid
(
)::text,
    session_token text not null unique,
    user_id text not null references identity.users
(
    id
) on delete cascade,
    ip_address inet,
    user_agent text,
    expires_at timestamptz not null,
    revoked_at timestamptz,
    created_at timestamptz not null default now
(
)
    );

create index if not exists idx_identity_sessions_user_id on identity.sessions(user_id);
create index if not exists idx_identity_sessions_active on identity.sessions(user_id, expires_at)
    where revoked_at is null;

create trigger trg_identity_users_updated_at
    before update
    on identity.users
    for each row execute function platform.touch_updated_at();
