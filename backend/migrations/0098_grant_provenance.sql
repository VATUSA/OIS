-- @formatter:off
-- Tell a machine-created grant from a human-created one (VATUSA/OIS#547).
--
-- Neither grant table carried any provenance: access.user_roles and access.user_permissions had the
-- user, the name, artcc_id, created_at and (for permissions) granted — no source, no granted_by.
-- Meanwhile both bulk writers are authoritative over an entire scope:
-- replace_user_permissions_scoped deletes everything at the scope and rewrites it, and the access
-- editor sets every assignable role held/not-held.
--
-- So a VATUSA mapper (#548) had only two options, both wrong: write authoritatively and silently undo
-- every manual grant on each sync, or write additively and never clean up after a demotion.
-- access.audit_logs cannot answer "is this row machine-owned?" — it is append-only, keyed by resource,
-- and pruned at 180 days.
--
-- Three sources:
--   manual  an admin set it in the access editor, or a group membership was set by hand
--   vatusa  the VATUSA sync owns it and may reconcile it away (#548)
--   system  OIS itself set it: the SERVER_ADMIN env reconciliation, and the USER baseline group
--
-- Note what provenance deliberately does NOT do: no reader consults it. The 0091 effective-permissions
-- view and fetch_effective_permissions ignore source entirely, so #543's single-resolver property is
-- untouched and a grant's authority never depends on who created it. Provenance answers only "whose
-- row is this to remove?".

alter table access.user_roles
    add column if not exists source text not null default 'manual'
        check (source in ('manual', 'vatusa', 'system'));

alter table access.user_permissions
    add column if not exists source text not null default 'manual'
        check (source in ('manual', 'vatusa', 'system'));

-- Everything that exists today was created by hand or by the baseline seed, so 'manual' is the correct
-- backfill and the default above applies it.
--
-- The default is then **removed**. It exists only for the backfill: leaving it would mean a writer that
-- forgets `source` silently claims to be a human grant, which on these two tables is precisely the
-- confusion this migration exists to end. Without a default, `not null` makes that insert fail
-- instead — the guarantee becomes the schema's rather than a convention someone has to remember.
alter table access.user_roles alter column source drop default;
alter table access.user_permissions alter column source drop default;

-- Sync reconciliation filters on (user, name, scope, source), so index the dimension it adds.
create index if not exists idx_access_user_roles_source
    on access.user_roles(source);
create index if not exists idx_access_user_permissions_source
    on access.user_permissions(source);

-- Let a manual grant and a synced grant of the same thing coexist.
--
-- The original uniqueness was (user, name, coalesce(artcc_id,'')), which excludes source — so one row
-- per scope, whoever wrote it first. That reopens the exact failure this migration closes: VATUSA
-- grants EC@ZDC, an admin "also" grants it, the existence check sees a row and does nothing, the row
-- stays 'vatusa', and a later demotion removes it along with the admin's intent. Silently.
--
-- Keying on source as well means each owner holds its own row: a demotion removes VATUSA's and the
-- manual grant survives. Two rows granting the same thing are harmless to resolution, which unions
-- them (0091) — but the two grant *readers* now need `distinct`, or the editor lists the entry twice.
drop index if exists access.idx_access_user_roles_scope;
create unique index if not exists idx_access_user_roles_scope
    on access.user_roles(user_id, role_name, coalesce(artcc_id, ''), source);

drop index if exists access.idx_access_user_permissions_scope;
create unique index if not exists idx_access_user_permissions_scope
    on access.user_permissions(user_id, permission_name, coalesce(artcc_id, ''), source);
