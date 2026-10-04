-- @formatter:off
-- VATUSA/OIS#607 (PR 1 of 4): a machine credential — a service account or a user's API key — can now
-- drive the TMU, flow and GDP writes, not just the release path #583 opened (0102).
--
-- Every attribution column these writes fill is a foreign key to identity.users, and a machine is not a
-- user, so each gains a nullable sibling naming the access.actors row instead. A user's write fills
-- both; a machine's fills only the actor column, so the row says a machine did it — never a person,
-- never nobody. Additive and unbackfilled, exactly like 0102: existing rows keep their user column, and
-- reads coalesce the two names.

alter table flow.fca
    add column if not exists created_by_actor text references access.actors(id) on delete set null,
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;

alter table flow.route
    add column if not exists created_by_actor text references access.actors(id) on delete set null,
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;

alter table tmu.gdp
    add column if not exists created_by_actor text references access.actors(id) on delete set null,
    add column if not exists updated_by_actor text references access.actors(id) on delete set null,
    add column if not exists published_by_actor text references access.actors(id) on delete set null;

alter table tmu.tmis
    add column if not exists created_by_actor text references access.actors(id) on delete set null,
    add column if not exists published_by_actor text references access.actors(id) on delete set null;

alter table tmu.programs
    add column if not exists created_by_actor text references access.actors(id) on delete set null,
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;

alter table tmu.ground_stops
    add column if not exists created_by_actor text references access.actors(id) on delete set null,
    add column if not exists updated_by_actor text references access.actors(id) on delete set null,
    add column if not exists published_by_actor text references access.actors(id) on delete set null;

alter table tmu.advisories
    add column if not exists created_by_actor text references access.actors(id) on delete set null,
    add column if not exists published_by_actor text references access.actors(id) on delete set null;
