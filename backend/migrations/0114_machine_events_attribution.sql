-- @formatter:off
-- VATUSA/OIS#607 (PR 2 of 4): a machine credential can now drive the events planning writes — DCC
-- requests, facility support, airport rates, TMI packages (create and deactivate), event capture and
-- debriefs. Each `updated_by` is a foreign key to identity.users and a machine is not a user, so each
-- table gains a nullable sibling naming the access.actors row, exactly as 0102 and 0110 did. A user's
-- write fills both; a machine's fills only the actor column. Additive and unbackfilled; reads coalesce.
--
-- events.tmi_package.updated_by is also the identity the event-package lifecycle job acts as when it
-- auto-publishes or auto-archives. Only a person arms or activates a package (those handlers stay
-- user-only until the job can act as a machine), and every writer keeps the pair in step.

alter table events.dcc_request
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table events.facility_support
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table events.airport_rate
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table events.tmi_package
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table stats.event_capture
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table events.event_debrief
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
