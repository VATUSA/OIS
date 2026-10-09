-- @formatter:off
-- VATUSA/OIS#805: re-tag the USER and SERVER_ADMIN memberships 0098 backfilled as `manual` to `system`.
--
-- 0098 marked every grant that existed then as `manual`, including the SERVER_ADMIN rows the env
-- reconciliation wrote and the baseline USER rows. Since then only OIS writes these two groups, always
-- as `system`: the login path (handlers::auth::ensure_user_login_access), while the access editor, the
-- group-member editor and VATUSA mappings all refuse a system group. So a `manual` row of either group
-- is a backfill. Left `manual`, it hid from the env reconciliation, which deleted only `system` rows,
-- so a CID removed from OIS_SERVER_ADMIN_CID kept SERVER_ADMIN for good; and the access reset, which
-- removes every `manual` row, would take a configured admin's role and older members' baseline.
--
-- No one's access changes here: resolution ignores `source` (0098). A server admin no longer
-- configured is demoted when the backend starts (handlers::auth::demote_unconfigured_server_admins).
--
-- The unique index includes `source` (0098), so a user who signed in after 0098 holds a `system` twin
-- beside the backfilled row. Updating that row would collide with the twin, so it is deleted instead.
-- Both statements match nothing on a second run.

delete from access.user_roles m
where m.source = 'manual'
  and m.role_name in ('USER', 'SERVER_ADMIN')
  and exists (
      select 1 from access.user_roles s
      where s.user_id = m.user_id
        and s.role_name = m.role_name
        and coalesce(s.artcc_id, '') = coalesce(m.artcc_id, '')
        and s.source = 'system'
  );

update access.user_roles
set source = 'system'
where source = 'manual'
  and role_name in ('USER', 'SERVER_ADMIN');
