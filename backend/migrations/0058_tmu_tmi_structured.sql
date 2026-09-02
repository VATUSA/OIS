-- @formatter:off
-- Structured NTML restriction TMIs. `restriction` stays the canonical raw line (typed directly, or
-- encoded from the structured form); `structured` holds the parsed NTML fields for a form-built TMI
-- (null for a raw-typed one), and `decoded` is its plain-English rendering that pilots read.

alter table tmu.tmis
    add column if not exists structured jsonb,
    add column if not exists decoded text;
