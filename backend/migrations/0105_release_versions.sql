-- @formatter:off
-- VATUSA/OIS#585: a release written by something other than OIS needs conflict detection. Each
-- release row carries a version that every write bumps; a writer can then say "only if none exists"
-- (`If-None-Match: *`) or "only if it is still version N" (`If-Match: N`), so a retried request
-- cannot issue a committed time twice and a stale one cannot silently replace a newer one.
-- Additive: existing rows start at version 1.

alter table flow.fca_release add column if not exists version bigint not null default 1;

alter table tmu.issued_cfrs add column if not exists version bigint not null default 1;
