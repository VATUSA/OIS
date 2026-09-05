-- @formatter:off
-- Issue #32: add access.self.read to the baseline self-service set so every member can view their
-- OWN permissions. New/demoted users get it via the login seed (BASELINE_SELF_SERVICE_PERMISSIONS
-- in handlers/auth.rs); this backfills existing baseline users — those already holding the granted
-- users.directory.read baseline grant — without touching anyone who already has a row for it (so an
-- explicit deny is left intact).

insert into access.user_permissions (user_id, permission_name, granted, artcc_id)
select up.user_id, 'access.self.read', true, null
from access.user_permissions up
where up.permission_name = 'users.directory.read'
  and up.granted
  and not exists (
      select 1 from access.user_permissions x
      where x.user_id = up.user_id
        and x.permission_name = 'access.self.read'
  );
