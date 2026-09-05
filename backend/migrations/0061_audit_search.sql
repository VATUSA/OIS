-- @formatter:off
-- Indexes backing audit-log search (issue #31): a free-text `q` filter (ILIKE across
-- action / resource / reason / actor) and a created_at date range, on top of the existing
-- exact filters. created_at also backs the default "newest first" ordering.

create extension if not exists pg_trgm;

create index if not exists idx_access_audit_logs_created_at
    on access.audit_logs (created_at desc);

-- Trigram GIN indexes accelerate the ILIKE '%q%' free-text search on the high-value columns.
create index if not exists idx_access_audit_logs_reason_trgm
    on access.audit_logs using gin (reason gin_trgm_ops);
create index if not exists idx_access_audit_logs_resource_id_trgm
    on access.audit_logs using gin (resource_id gin_trgm_ops);
