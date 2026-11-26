-- @formatter:off
-- VATUSA/OIS#1121: CONTROLLER applies nationally, and every signed-in member reads FCAs.
--
-- The owner's decision on #1121: a rostered VATUSA controller, home or visiting, uses the flow tools at
-- every ARTCC, except event planning and TMU publishing, which CONTROLLER does not hold (0126, 0129).
-- The VATUSA sync now grants CONTROLLER once, nationally, to a member with a home or visiting ARTCC
-- (repos::vatusa::desired_vatusa_grants) in place of one row per ARTCC. This converts the rows it wrote
-- before, so the change applies at deploy and not at each member's next sync.
--
-- Only `source = 'vatusa'` rows of members still on role sync are converted. A member an admin detached
-- (#549) keeps their rows as they are until a Resync, which then applies the new rule; a hand-made
-- (`manual`) CONTROLLER grant at an ARTCC is never touched.
--
-- Rewrites rows: the scoped vatusa rows are deleted and not recoverable from this file. If wrong, the
-- next VATUSA sync of the member rebuilds their rows from identity.users and identity.vatusa_visits.
-- backend/audits/0139_national_controller_effect.sql lists the members this changes.
--
-- USER (every signed-in member, BASELINE_ROLE) gains flow.fca.read, so FCAs and IDST are readable
-- by everyone signed in. Every FCA write stays on flow.fca.update / flow.fca.delete.

update access.roles
set description = 'Rostered controller: baseline operational access at every ARTCC, granted by VATUSA sync'
where name = 'CONTROLLER';

insert into access.user_roles (user_id, role_name, artcc_id, source)
select distinct ur.user_id, 'CONTROLLER', null::text, 'vatusa'
from access.user_roles ur
join identity.users u on u.id = ur.user_id
where ur.role_name = 'CONTROLLER'
  and ur.source = 'vatusa'
  and ur.artcc_id is not null
  and u.vatusa_roles_detached_at is null
on conflict do nothing;

delete from access.user_roles ur
using identity.users u
where u.id = ur.user_id
  and ur.role_name = 'CONTROLLER'
  and ur.source = 'vatusa'
  and ur.artcc_id is not null
  and u.vatusa_roles_detached_at is null;

insert into access.role_permissions (role_name, permission_name)
select 'USER', p.name
from access.permissions p
where p.name = 'flow.fca.read'
on conflict do nothing;
