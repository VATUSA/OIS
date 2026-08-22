-- User-owned API keys (personal access tokens) for third-party integration.
--
-- A key is a capability strictly bounded by its owner's LIVE permissions: at request time its
-- effective authority is `granted set ∩ owner's current effective permissions/scope` (computed in
-- the backend, not frozen here). So demoting or deactivating the owner instantly narrows their keys.
--
-- Secrets are stored SHA-256-hashed only (the token is high-entropy random), never in plaintext.
-- One secret per key; rotation overwrites the hash in place.

create table if not exists access.api_keys (
    id text primary key default gen_random_uuid()::text,
    owner_user_id text not null references identity.users(id) on delete cascade,
    name text not null,
    description text,
    -- Public, non-secret identifier shown in listings, e.g. `ois_pat_a1b2c3d4`.
    prefix text not null,
    -- SHA-256 hex of the full token; the lookup key on bearer auth.
    secret_hash text not null unique,
    status text not null default 'active' check (status in ('active', 'disabled')),
    expires_at timestamptz,
    last_used_at timestamptz,
    last_used_ip inet,
    revoked_at timestamptz,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists idx_access_api_keys_owner on access.api_keys(owner_user_id);

create trigger trg_access_api_keys_updated_at
before update on access.api_keys
for each row execute function platform.touch_updated_at();

-- The key's granted (permission, scope) subset. Grant-only (no deny rows); `artcc_id NULL` = national.
-- Mirrors access.user_permissions so the same subsetting semantics apply.
create table if not exists access.api_key_permissions (
    id text primary key default gen_random_uuid()::text,
    api_key_id text not null references access.api_keys(id) on delete cascade,
    permission_name text not null references access.permissions(name),
    artcc_id text,
    created_at timestamptz not null default now()
);

create unique index if not exists uq_access_api_key_permissions
    on access.api_key_permissions(api_key_id, permission_name, coalesce(artcc_id, ''));

create index if not exists idx_access_api_key_permissions_key
    on access.api_key_permissions(api_key_id);

-- Audit attribution: api keys become a first-class actor type so every mutation a key makes is
-- logged with the key identified (owner CID + key prefix).
alter table access.actors drop constraint if exists actors_actor_type_check;
alter table access.actors add constraint actors_actor_type_check
    check (actor_type in ('user', 'service_account', 'system', 'api_key'));
alter table access.actors
    add column if not exists api_key_id text references access.api_keys(id) on delete cascade;

-- Permissions catalog: self-service create/manage-your-own, plus grantable oversight over all keys.
-- SERVER_ADMIN holds all three implicitly via access.v_effective_user_permissions.
insert into access.permissions (name, description) values
    ('api_keys.key.create', 'Create and manage your own API keys'),
    ('api_keys.key.read',   'View any user''s API keys (oversight)'),
    ('api_keys.key.delete', 'Revoke any user''s API keys (oversight)')
on conflict (name) do nothing;
