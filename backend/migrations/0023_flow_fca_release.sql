-- @formatter:off
-- Frozen CFR releases per FCA: when a controller issues a release for a crossing
-- aircraft, its metered crossing time (cta) and wheels-up/release time (edct) are
-- pinned. The metering engine treats a released aircraft as a fixed constraint;
-- unreleased ground traffic floats around it. Ported from vatflow's `fca.releases`.

create table if not exists flow.fca_release (
    fca_id text not null references flow.fca(id) on delete cascade,
    callsign text not null,
    cta_ms bigint not null,      -- frozen metered crossing time, epoch ms
    edct_ms bigint not null,     -- release / wheels-up time, epoch ms
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (fca_id, callsign)
);

-- Manual sequence support (drag-to-reorder): an ordered callsign list.
alter table flow.fca add column if not exists manual_order text[] not null default '{}';
alter table flow.fca add column if not exists manual_seq boolean not null default false;
