-- @formatter:off
-- VATUSA/OIS#583: a machine credential (a service account or a user's API key) can now mark, swap and
-- issue releases. The existing attribution columns are foreign keys to `identity.users`, and a machine
-- is not a user, so each table the release path writes gains a nullable sibling naming the
-- `access.actors` row instead. A user's write fills both; a machine's fills only the actor column.
-- Additive and unbackfilled: existing rows keep their user column, and reads coalesce the two.

alter table flow.fca_release
    add column if not exists updated_by_actor text references access.actors(id) on delete set null;

alter table tmu.issued_cfrs
    add column if not exists issued_by_actor text references access.actors(id) on delete set null;
