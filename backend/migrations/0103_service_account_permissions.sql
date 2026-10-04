-- @formatter:off
-- VATUSA/OIS#584: a service account can be granted individual permissions at individual ARTCCs, so
-- onboarding an integration no longer needs a migration. Until now it could hold only roles, and the
-- only role that granted a machine anything was BOT.
--
-- Mirrors access.api_key_permissions (0045): grant-only, `artcc_id NULL` = national. The gate and the
-- handler-side scope both read roles ∪ these rows (repos/access.rs), so they cannot disagree.
create table if not exists access.service_account_permissions (
    id text primary key default gen_random_uuid()::text,
    service_account_id text not null references access.service_accounts(id) on delete cascade,
    permission_name text not null references access.permissions(name),
    artcc_id text,
    created_at timestamptz not null default now()
);

create unique index if not exists uq_access_service_account_permissions
    on access.service_account_permissions(service_account_id, permission_name, coalesce(artcc_id, ''));

create index if not exists idx_access_service_account_permissions_account
    on access.service_account_permissions(service_account_id);

-- Credentials now carry a lifetime (default 90 days, at most 365 — handlers/service_accounts.rs).
-- Existing live credentials were issued without one; give them the maximum from now, so none breaks
-- on deploy and none outlives the cap.
update access.service_account_credentials
    set expires_at = now() + interval '365 days'
    where expires_at is null and revoked_at is null;
