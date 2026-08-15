-- @formatter:off
-- Time-varying AAR: a GDP can step its acceptance rate across the window (e.g. 30/hr then
-- 45/hr). Stored as an array of {start_time: "HHMM", aar: int}; empty = flat rate.

alter table tmu.gdp
    add column if not exists aar_steps jsonb not null default '[]'::jsonb;
