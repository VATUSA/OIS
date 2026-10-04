-- Read-only audit for migration 0099 (VATUSA/OIS#550): proves it changes no one's effective access.
--
-- AC 3 of #550 is "no user's effective permissions change. Verified by computing effective sets
-- before and after." 0099 deletes direct grants a held group already gives, and runs on startup, so
-- this is the check to run against real data before it ships.
--
-- It does not re-implement the migration. It applies 0099 itself inside a transaction, compares
-- effective access before and after, and rolls back.
--
-- Run it from the repo root, against a production snapshot on which 0099 has NOT been applied:
--
--   psql "$SNAPSHOT_URL" -v ON_ERROR_STOP=1 -f backend/audits/0099_collapse_effect.sql
--
-- Nothing persists: the transaction always rolls back.
--
-- ## Why it evaluates access rather than diffing rows
--
-- A raw diff of `v_effective_user_permissions` would report every collapsed row as lost — removing a
-- ZDC grant held under a national membership deletes the `(user, p, ZDC)` tuple without changing what
-- the user can do. So `effective_now` answers the question the resolver answers: for each user and
-- permission, is it allowed **nationally** (`scope` NATIONAL), and at **each ARTCC** that appears
-- anywhere in the access tables. The rule is `repos::access::compose`'s, stated in 0091: a grant
-- applies at its own ARTCC or, if national, everywhere; a deny removes the permission at its own ARTCC
-- or, if national, everywhere — even where a scoped grant exists. An ARTCC no row mentions is allowed
-- exactly when NATIONAL is, so it needs no row of its own.
--
-- Reading the result:
--   * `rows_removed` — how many direct grants 0099 deletes. Informational.
--   * the diff — **should be empty.** `lost` means a user can no longer do something at that scope;
--     `gained` cannot happen from deletions alone and would mean the comparison itself is wrong.
--
-- `repos::access::tests::the_0099_audit_reports_no_change_for_the_shipped_migration` and
-- `..._catches_a_migration_that_drops_bespoke_grants` run the SETUP and DIFF blocks below, verbatim,
-- so this file cannot drift from what the tests prove.

begin;

-- BEGIN SETUP
create temp table audit_direct_before as
select count(*) as n from access.user_permissions;

create temp table audit_scopes as
select null::text as artcc_id
union
select artcc_id from access.user_permissions where artcc_id is not null
union
select artcc_id from access.user_roles where artcc_id is not null;

create temp view effective_now as
select pr.user_id, pr.permission_name, coalesce(s.artcc_id, 'NATIONAL') as scope
from (select distinct user_id, permission_name from access.v_effective_user_permissions) pr
cross join audit_scopes s
where exists (
        select 1 from access.v_effective_user_permissions g
        where g.user_id = pr.user_id and g.permission_name = pr.permission_name and g.granted
          and (g.artcc_id is null or g.artcc_id = s.artcc_id))
  and not exists (
        select 1 from access.v_effective_user_permissions d
        where d.user_id = pr.user_id and d.permission_name = pr.permission_name and not d.granted
          and (d.artcc_id is null or d.artcc_id = s.artcc_id));

create temp table effective_before as
select * from effective_now;
-- END SETUP
;

\i backend/migrations/0099_collapse_redundant_direct_grants.sql

select (select n from audit_direct_before) - count(*) as rows_removed from access.user_permissions;

-- BEGIN DIFF
select d.change, d.user_id, u.display_name, d.permission_name, d.scope
from (
    (select 'gained' as change, user_id, permission_name, scope from effective_now
     except
     select 'gained', user_id, permission_name, scope from effective_before)
    union all
    (select 'lost', user_id, permission_name, scope from effective_before
     except
     select 'lost', user_id, permission_name, scope from effective_now)
) d
join identity.users u on u.id = d.user_id
order by d.change, u.display_name, d.permission_name, d.scope
-- END DIFF
;

rollback;
