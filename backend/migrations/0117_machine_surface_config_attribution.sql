-- @formatter:off
-- VATUSA/OIS#607 (PR 3 of 4): a machine credential can now write airport configurations and surface
-- data (gates, ramp areas, taxiways, runways). Each `updated_by` is a foreign key to identity.users and
-- a machine is not a user, so each table gains a nullable sibling naming the access.actors row, as
-- 0110 and 0114 did. A user's write fills both; a machine's fills only the actor column. Additive and
-- unbackfilled; the config read coalesces the two names.

alter table flow.airport_config
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table flow.airport_gate
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table flow.airport_ramp_area
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table flow.airport_taxiway
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
alter table flow.airport_runway
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;
