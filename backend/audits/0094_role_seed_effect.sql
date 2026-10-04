-- Read-only audit for migration 0094 (VATUSA/OIS#544): exactly whose effective permissions it changes.
--
-- AC 4 of #544 is "no user's effective permissions change when this migration runs, verified against
-- real users". Seeding EC, AEC, EVENTS_TEAM and VATUSA_STAFF — which granted nothing before — re-grants
-- anything an admin unticked for a holder of those roles, because the access editor stores an unticked
-- box as an absent row rather than a deny. This lists every such change, so the decision to ship is
-- made against the real population instead of an assumption.
--
-- It does not re-implement the seed or the resolver. It applies 0094 itself inside a transaction,
-- diffs `access.v_effective_user_permissions` before and after, and rolls back.
--
-- Run it from the repo root, against a production snapshot on which 0094 has NOT been applied:
--
--   psql "$SNAPSHOT_URL" -v ON_ERROR_STOP=1 -f backend/audits/0094_role_seed_effect.sql
--
-- A snapshot rather than production because 0094 takes row locks on the access tables while it runs.
-- Nothing persists either way: the transaction always rolls back.
--
-- Reading the result:
--   * `gained` — a permission the user did not hold at that scope before and does after. For the seeded
--     roles this is the escalation: something an admin deliberately took away coming back.
--   * `lost`   — should be empty. A row here means the baseline cleanup removed something the `USER`
--     group did not replace.
-- The diff compares raw grant tuples and ignores deny precedence, so it may over-report a gain that a
-- deny row would cancel. It never under-reports.
--
-- `repos::access::tests::the_0094_audit_reports_exactly_who_the_seed_would_change` runs the snapshot and
-- diff blocks below, verbatim, so this file cannot drift from what the test proves.

begin;

-- BEGIN SNAPSHOT
create temp table effective_before on commit drop as
select user_id, permission_name, artcc_id
from access.v_effective_user_permissions
where granted
-- END SNAPSHOT
;

\i backend/migrations/0094_seed_role_permissions.sql

-- BEGIN DIFF
select d.change, d.user_id, u.display_name, d.permission_name, coalesce(d.artcc_id, 'NATIONAL') as scope
from (
    (select 'gained' as change, user_id, permission_name, artcc_id
     from access.v_effective_user_permissions where granted
     except
     select 'gained', user_id, permission_name, artcc_id from effective_before)
    union all
    (select 'lost', user_id, permission_name, artcc_id from effective_before
     except
     select 'lost', user_id, permission_name, artcc_id
     from access.v_effective_user_permissions where granted)
) d
join identity.users u on u.id = d.user_id
order by d.change, u.display_name, d.permission_name, scope
-- END DIFF
;

rollback;
