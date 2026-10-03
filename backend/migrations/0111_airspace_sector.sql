-- ATC sector volumes (#594), for the Airspace Monitor (#593). No public FAA or vNAS source
-- publishes altitude-bounded sector geometry, so OIS owns this dataset. It is filled offline by
-- `cargo run -p ois-backend --bin airspace-sector-importer`, never at runtime.
--
-- One row per volume: a sector can be several volumes, so `volume_id` (the source's id for the
-- piece) is what is unique within an ARTCC. Geometry follows 0067: plain jsonb, an array of closed
-- rings of [lat, lon] pairs (one ring per polygon part; no holes). Ring topology is validated by
-- the one write path (`repos::airspace_sectors::replace_artcc`), not here.
--
-- Provenance is recorded per row so the data can be traced and regenerated: `source` names the
-- dataset and `source_cycle` the exact revision it was imported from.

create table flow.airspace_sector (
    id           text primary key default gen_random_uuid()::text,
    artcc        text not null,
    sector_id    text not null,
    volume_id    text not null,
    name         text,
    tier         text not null check (tier in ('low', 'high', 'ultra_high', 'approach')),
    base_alt_ft  integer not null check (base_alt_ft >= 0),
    top_alt_ft   integer not null,
    rings        jsonb not null,
    source       text not null,
    source_cycle text not null,
    imported_at  timestamptz not null default now(),
    unique (artcc, volume_id),
    check (base_alt_ft < top_alt_ft)
);

create index if not exists idx_airspace_sector_artcc on flow.airspace_sector(artcc);
