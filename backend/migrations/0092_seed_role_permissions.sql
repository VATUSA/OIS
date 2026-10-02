-- @formatter:off
-- Give the assignable roles real permission sets, so a role name means something (VATUSA/OIS#544).
--
-- Before this, access.role_permissions held 9 rows across 5 roles in 91 migrations, and four of the
-- seven assignable roles — VATUSA_STAFF, EVENTS_TEAM, EC, AEC — bundled nothing at all. Every
-- capability therefore arrived as a per-user direct row, expanded from the presets in
-- web/src/lib/presets.ts, which then drifted from the preset definition permanently.
--
-- The mapping below is not invented: it is exactly what those presets grant today, read from
-- presets.ts. Each preset names a role and a set of first-segment domains, and expands with
-- `permission_name.split('.')[0] in domains` — which is the split_part() filter used here.
--
-- Defaults, not a contract. Sub-issue C adds the UI to edit a group's set; these rows are a starting
-- point. Note what that trades away: the presets resolved their domain rule against the LIVE catalog,
-- so a permission added tomorrow reached them automatically, while these rows do not. The test
-- `seeded_roles_match_the_preset_domains` fails when a new permission appears, which turns that
-- silent drift into a decision someone has to make.

-- VATUSA_STAFF: every permission, nationally. SERVER_ADMIN is excluded on purpose — it holds the
-- catalogue implicitly through the view's cross join and must stay env-bootstrapped.
insert into access.role_permissions (role_name, permission_name)
select 'VATUSA_STAFF', name from access.permissions
on conflict do nothing;

-- Day-to-day operational domains. DCC_STAFF nationally; EC and AEC scoped to a facility by their
-- membership row (role_permissions carries no scope — scope lives on access.user_roles.artcc_id,
-- which is what lets one EC role mean "EC at ZDC" for one person and national for another).
insert into access.role_permissions (role_name, permission_name)
select r.role_name, p.name
from (values ('DCC_STAFF'), ('EC'), ('AEC')) as r(role_name)
cross join access.permissions p
where split_part(p.name, '.', 1) in ('tmu', 'flow', 'events', 'ace', 'stats')
on conflict do nothing;

-- NTMO: traffic management only.
insert into access.role_permissions (role_name, permission_name)
select 'NTMO', name from access.permissions
where split_part(name, '.', 1) in ('tmu', 'flow', 'stats')
on conflict do nothing;

-- EVENTS_TEAM: event coordination.
insert into access.role_permissions (role_name, permission_name)
select 'EVENTS_TEAM', name from access.permissions
where split_part(name, '.', 1) in ('events', 'ace')
on conflict do nothing;

-- ACE: support requests.
insert into access.role_permissions (role_name, permission_name)
select 'ACE', name from access.permissions
where split_part(name, '.', 1) = 'ace'
on conflict do nothing;

-- The signed-in baseline becomes the USER group's content rather than five direct rows per user.
--
-- USER has existed since 0004 with one permission and was granted to nobody: the login path wrote
-- BASELINE_SELF_SERVICE_PERMISSIONS as direct access.user_permissions rows instead, and
-- presets.ts carried a hand-maintained "mirror" of the USER role that was not in fact a mirror —
-- one name there, five different ones in Rust. Both are now gone; this is the single definition.
insert into access.role_permissions (role_name, permission_name)
values
    ('USER', 'auth.profile.read'),
    ('USER', 'auth.profile.update'),
    ('USER', 'auth.sessions.delete'),
    ('USER', 'access.self.read'),
    ('USER', 'users.directory.read')
on conflict do nothing;

-- Grant USER to everyone who already has an account. Without this the rows above reach nobody,
-- because nothing has ever granted the role.
insert into access.user_roles (user_id, role_name)
select u.id, 'USER'
from identity.users u
where not exists (
    select 1 from access.user_roles ur
    where ur.user_id = u.id and ur.role_name = 'USER' and ur.artcc_id is null
);

-- And remove the direct rows the group now supplies. Deliberately narrow: only these five names, at
-- national scope, and only where the user actually holds the USER role — so this cannot strip access
-- from someone the grant above missed. The ~80 preset-expanded rows per user are NOT touched; that
-- cleanup belongs to sub-issue H, which owns it.
delete from access.user_permissions up
where up.artcc_id is null
  and up.granted is true
  and up.permission_name in (
      'auth.profile.read',
      'auth.profile.update',
      'auth.sessions.delete',
      'access.self.read',
      'users.directory.read'
  )
  and exists (
      select 1 from access.user_roles ur
      where ur.user_id = up.user_id and ur.role_name = 'USER' and ur.artcc_id is null
  );
