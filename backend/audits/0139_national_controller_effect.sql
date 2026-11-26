-- Read-only audit for the #1121 deploy (VATUSA/OIS#1121): which members migration 0139 changes.
--
-- 0139 turns each member's ARTCC-scoped `vatusa` CONTROLLER rows into one national `vatusa` row,
-- skipping members detached from role sync (#549), and gives USER `flow.fca.read`. It applies 0139
-- itself inside a transaction, counts before and after, and rolls back. Counts only: no CID or name.
--
-- Run it from the repo root, against a production snapshot on which 0139 has NOT been applied:
--
--   psql "$SNAPSHOT_URL" -v ON_ERROR_STOP=1 -f backend/audits/0139_national_controller_effect.sql
--
-- Reading the result:
--   * `scoped_vatusa_rows` falls to the detached members' rows only, `national_vatusa_rows` rises to
--     one per converted member, and `members_with_controller` does not change: nobody loses the role.
--   * `detached_scoped_rows` is the same before and after.
--   * `user_reads_fcas` goes from false to true.

begin;

create temp view audit_0139 as
select
    (select count(*) from access.user_roles
      where role_name = 'CONTROLLER' and source = 'vatusa' and artcc_id is not null) as scoped_vatusa_rows,
    (select count(*) from access.user_roles
      where role_name = 'CONTROLLER' and source = 'vatusa' and artcc_id is null) as national_vatusa_rows,
    (select count(distinct user_id) from access.user_roles
      where role_name = 'CONTROLLER') as members_with_controller,
    (select count(*) from access.user_roles r join identity.users u on u.id = r.user_id
      where r.role_name = 'CONTROLLER' and r.source = 'vatusa' and r.artcc_id is not null
        and u.vatusa_roles_detached_at is not null) as detached_scoped_rows,
    exists(select 1 from access.role_permissions
            where role_name = 'USER' and permission_name = 'flow.fca.read') as user_reads_fcas;

create temp table audit_0139_before as select * from audit_0139;

\i backend/migrations/0139_national_controller.sql

select 'before' as at, * from audit_0139_before
union all
select 'after', * from audit_0139;

rollback;
