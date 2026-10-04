-- Sector consolidation (#599, Airspace Monitor #593): sectors worked at another sector's position. A
-- consolidated sector has no Monitor row of its own; its airspace counts in the target's row as a union
-- (`feed::monitor::sector_loads`), under the target's alert parameter.
--
-- Shared, not per-browser. Always flat — a target is never itself a source — which the one writer
-- (`repos::sector_consolidations::consolidate`) keeps true in a single transaction. Same ARTCC only, by
-- construction: one artcc column.

create table flow.sector_consolidation (
    artcc            text not null,
    sector_id        text not null,
    target_sector_id text not null,
    updated_by       text references identity.users(id) on delete set null,
    updated_at       timestamptz not null default now(),
    primary key (artcc, sector_id),
    check (sector_id <> target_sector_id)
);
