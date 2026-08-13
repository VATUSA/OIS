-- @formatter:off
-- Reshape TMIs to the NTML/FAA row model: a requesting + providing facility, the
-- restriction, and a start/stop time window. Drops the earlier kind/element/reason/scope
-- columns in favour of the requesting↔providing pair controllers actually log.

drop index if exists idx_tmu_tmis_artcc;

alter table tmu.tmis
    drop column if exists artcc_id,
    drop column if exists kind,
    drop column if exists element,
    drop column if exists reason,
    add column if not exists requesting text not null default '',
    add column if not exists providing text not null default '';

-- Defaults were only needed to backfill any existing rows during the add above.
alter table tmu.tmis
    alter column requesting drop default,
    alter column providing drop default;

alter table tmu.tmis rename column effective_start to start_time;
alter table tmu.tmis rename column effective_end to stop_time;
