-- @formatter:off
-- Access control, ported from osmium with a per-ARTCC scope dimension added from
-- day one: `artcc_id` on user_roles / user_permissions. NULL = national scope.
-- Scope enforcement rolls out per-domain; the effective-permissions view treats all
-- grants as applicable for now (Phase 0 users are national).

create table if not exists access.roles (
    name text primary key,
    description text,
    is_system boolean not null default true,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create table if not exists access.permissions (
    name text primary key,
    description text,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create table if not exists access.role_permissions (
    role_name text not null references access.roles(name) on delete cascade,
    permission_name text not null references access.permissions(name) on delete cascade,
    created_at timestamptz not null default now(),
    primary key (role_name, permission_name)
);

-- Surrogate id + a uniqueness index over coalesce(artcc_id, '') so the same role can
-- be granted at multiple ARTCCs (NULL national) without NULLs breaking a composite PK.
create table if not exists access.user_roles (
    id text primary key default gen_random_uuid()::text,
    user_id text not null references identity.users(id) on delete cascade,
    role_name text not null references access.roles(name) on delete cascade,
    artcc_id text,
    created_at timestamptz not null default now()
);

create unique index if not exists idx_access_user_roles_scope
    on access.user_roles(user_id, role_name, coalesce(artcc_id, ''));
create index if not exists idx_access_user_roles_user_id on access.user_roles(user_id);

create table if not exists access.user_permissions (
    id text primary key default gen_random_uuid()::text,
    user_id text not null references identity.users(id) on delete cascade,
    permission_name text not null references access.permissions(name) on delete cascade,
    granted boolean not null default true,
    artcc_id text,
    created_at timestamptz not null default now()
);

create unique index if not exists idx_access_user_permissions_scope
    on access.user_permissions(user_id, permission_name, coalesce(artcc_id, ''));
create index if not exists idx_access_user_permissions_user_id on access.user_permissions(user_id);

-- Machine clients (the Discord bot). Bearer credentials are stored hashed.
create table if not exists access.service_accounts (
    id text primary key default gen_random_uuid()::text,
    key text not null unique,
    name text not null,
    description text,
    status text not null default 'active' check (status in ('active', 'disabled')),
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create table if not exists access.service_account_credentials (
    id text primary key default gen_random_uuid()::text,
    service_account_id text not null references access.service_accounts(id) on delete cascade,
    credential_type text not null default 'bearer_token'
        check (credential_type in ('api_key', 'bearer_token', 'oauth_client_secret')),
    secret_hash text not null,
    last_used_at timestamptz,
    expires_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz not null default now()
);

create table if not exists access.service_account_roles (
    id text primary key default gen_random_uuid()::text,
    service_account_id text not null references access.service_accounts(id) on delete cascade,
    role_name text not null references access.roles(name) on delete cascade,
    artcc_id text,
    starts_at timestamptz not null default now(),
    ends_at timestamptz,
    created_at timestamptz not null default now()
);

create table if not exists access.actors (
    id text primary key default gen_random_uuid()::text,
    actor_type text not null check (actor_type in ('user', 'service_account', 'system')),
    user_id text references identity.users(id) on delete cascade,
    service_account_id text references access.service_accounts(id) on delete cascade,
    display_name text not null,
    created_at timestamptz not null default now()
);

create table if not exists access.audit_logs (
    id text primary key default gen_random_uuid()::text,
    actor_id text references access.actors(id) on delete set null,
    action text not null,
    resource_type text not null,
    resource_id text,
    artcc_id text,
    reason text,
    before_state jsonb,
    after_state jsonb,
    ip_address inet,
    created_at timestamptz not null default now()
);

create index if not exists idx_access_audit_logs_resource on access.audit_logs(resource_type, resource_id);

-- Highest-priority role for display (SERVER_ADMIN wins, then facility/staff, then USER).
create or replace view access.v_user_primary_role as
select
    ur.user_id,
    (
        array_agg(
            ur.role_name
            order by
                case
                    when ur.role_name = 'SERVER_ADMIN' then 0
                    when ur.role_name = 'USER' then 2
                    else 1
                end,
                ur.role_name
        )
    )[1] as primary_role
from access.user_roles ur
group by ur.user_id;

-- Effective permissions: role-derived UNION server-admin(all perms) UNION direct
-- grants, MINUS explicit denies. Scope (artcc_id) is ignored here for now.
create or replace view access.v_effective_user_permissions as
with server_admin_users as (
    select distinct user_id from access.user_roles where role_name = 'SERVER_ADMIN'
),
role_permissions as (
    select distinct ur.user_id, rp.permission_name
    from access.user_roles ur
    join access.role_permissions rp on rp.role_name = ur.role_name
),
server_admin_permissions as (
    select sau.user_id, p.name as permission_name
    from server_admin_users sau
    cross join access.permissions p
),
granted_permissions as (
    select up.user_id, up.permission_name
    from access.user_permissions up
    where up.granted is true
),
denied_permissions as (
    select up.user_id, up.permission_name
    from access.user_permissions up
    where up.granted is false
),
candidate_permissions as (
    select * from role_permissions
    union
    select * from server_admin_permissions
    union
    select * from granted_permissions
)
select cp.user_id, cp.permission_name
from candidate_permissions cp
left join denied_permissions dp
    on dp.user_id = cp.user_id
   and dp.permission_name = cp.permission_name
where dp.user_id is null;

create trigger trg_access_roles_updated_at
before update on access.roles
for each row execute function platform.touch_updated_at();

create trigger trg_access_permissions_updated_at
before update on access.permissions
for each row execute function platform.touch_updated_at();

create trigger trg_access_service_accounts_updated_at
before update on access.service_accounts
for each row execute function platform.touch_updated_at();
