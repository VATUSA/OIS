-- @formatter:off
-- VATUSA/OIS#607 (PR 4 of 4): the last handlers that refused a machine credential now admit one.
--
-- 1. Each write they make names the machine. `*_by` is a foreign key to identity.users and a machine is
--    not a user, so each table gains a nullable sibling naming the access.actors row (0110/0114/0117).
--    Additive and unbackfilled; reads coalesce the two names.
alter table flow.manual_flight_exclusion
    add column if not exists created_by_actor text references access.actors(id) on delete set null;
alter table flow.aircraft_profile
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table flow.facility_map_config
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table flow.runway_config
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table flow.runway_saved_config
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table stats.capture
    add column if not exists created_by_actor text references access.actors(id) on delete set null;
alter table ace.requests
    add column if not exists decided_by_actor text references access.actors(id) on delete set null;

-- 2. The event-package lifecycle job now acts as a package's `updated_by_actor` rather than its
--    `updated_by`, so a key or a service account can arm one. 0114 added that column unbackfilled, so a
--    package armed or activated before it has a person in `updated_by` and no actor — and the job would
--    silently stop publishing or archiving it. Name the same person's actor, creating it exactly as
--    `resolve_user_actor_id` would. Only rows with a person and no actor are touched.
insert into access.actors (actor_type, user_id, display_name)
select distinct 'user', u.id, u.display_name
from events.tmi_package p
join identity.users u on u.id = p.updated_by
where p.updated_by_actor is null
  and not exists (
      select 1 from access.actors a where a.actor_type = 'user' and a.user_id = u.id
  );

update events.tmi_package p
set updated_by_actor = (
    select a.id from access.actors a
    where a.actor_type = 'user' and a.user_id = p.updated_by
    order by a.created_at
    limit 1
)
where p.updated_by is not null
  and p.updated_by_actor is null;
