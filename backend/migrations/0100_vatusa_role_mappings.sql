-- @formatter:off
-- Map VATUSA staff roles to OIS groups (VATUSA/OIS#548).
--
-- identity.vatusa_roles is refreshed on every login, roster webhook and reconcile tick, but nothing
-- ever read it into access.*. VATUSA's vocabulary (ATM, DATM, TA, EC, AEC, WM, FE, INS, MTR) barely
-- overlaps OIS's groups, so the bridge is a configurable table rather than a hardcoded match.
--
-- A row means: a member holding `vatusa_role` (at `facility`, or at any facility when it is null) is
-- granted the OIS group `role_name`. The grant is scoped to the facility the VATUSA role is held at;
-- a division role (facility ZHQ, not an ARTCC) maps to a NATIONAL grant (artcc_id null), because a
-- division role is national. A VATUSA facility that is neither ZHQ nor in org.facilities is skipped —
-- it can never reach the access.user_roles FK.
--
-- Grants written from this table carry source = 'vatusa' (0098), so the reconciler adds and removes
-- only its own rows and never touches anything granted by hand.
--
-- Deliberately seeded with NOTHING: VATUSA becoming a source of OIS access is a policy decision, made
-- one mapping at a time by a human in the admin UI, not by a migration. Until then the reconciler runs
-- and grants nobody anything.

create table if not exists access.vatusa_role_mappings (
    id bigint generated always as identity primary key,
    vatusa_role text not null,
    facility text,
    role_name text not null references access.roles(name) on delete cascade,
    created_at timestamptz not null default now()
);

create unique index if not exists idx_vatusa_role_mappings_unique
    on access.vatusa_role_mappings(vatusa_role, coalesce(facility, ''), role_name);

-- The audit actor every sync-driven grant change is attributed to, so the dossier reads "VATUSA sync"
-- rather than an anonymous row.
insert into access.actors (id, actor_type, display_name)
values ('vatusa-sync', 'system', 'VATUSA sync')
on conflict (id) do nothing;
