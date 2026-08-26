-- The ACE team is sourced from VATUSA, not maintained in OIS. Remove the local roster and its perms;
-- claiming stays gated by ace.requests.claim (unaffected). The per-facility EC for DCC threads now
-- comes from access control (the EC role scoped to the facility), not a stored list.

drop table if exists ace.team_members;

delete from access.role_permissions where permission_name in ('ace.team.read', 'ace.team.update');
delete from access.permissions where name in ('ace.team.read', 'ace.team.update');
