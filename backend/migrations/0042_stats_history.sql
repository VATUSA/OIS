-- @formatter:off
-- Historical replay support for the time-machine dashboard: persist winds-aloft snapshots so past
-- ETAs are accurate, and retain traffic-management entities (TMIs / GDPs / ground stops / FCAs)
-- long enough to reconstruct which were active at a past instant `T`.
--
-- Retention model (no separate history tables — the live rows ARE the history):
--   * TMIs/GDPs/ground stops already carry `status` (draft→published→expired/cancelled) +
--     `published_at`. We add `ended_at` to record an EARLY cancellation (natural expiry is already
--     bounded by each entity's operational window). The cleanup job stops hard-deleting rows that
--     were ever published; a row is "active at T" iff it was published (`published_at <= T`), its
--     window still covers T, and it wasn't cancelled before T (`ended_at is null or ended_at > T`).
--     Rows that were NEVER published (drafts) are still removed — per the rule that a TMI created
--     and dropped without publishing never happened.
--   * FCAs have no publish step, so they get a plain `deleted_at` soft-delete; "active at T" =
--     existed at T and not yet deleted.

-- Winds-aloft snapshots — one serialized `Winds` per refresh (hourly). jsonb so Postgres TOASTs it;
-- a snapshot is a few KB. Pruned with the position time-series on the same retention window.
create table if not exists stats.winds (
    ts   timestamptz primary key,
    data jsonb       not null
);

-- Early-cancellation timestamp (null = ran to natural expiry or still active).
alter table tmu.tmis         add column if not exists ended_at timestamptz;
alter table tmu.ground_stops add column if not exists ended_at timestamptz;
alter table tmu.gdp          add column if not exists ended_at timestamptz;

-- Soft-delete for FCAs (null = live).
alter table flow.fca add column if not exists deleted_at timestamptz;

-- Replay lookups scan by the activation window; small partial indexes keep them cheap.
create index if not exists idx_tmu_tmis_published on tmu.tmis (published_at) where published_at is not null;
create index if not exists idx_tmu_gs_published on tmu.ground_stops (published_at) where published_at is not null;
create index if not exists idx_tmu_gdp_published on tmu.gdp (published_at) where published_at is not null;
create index if not exists idx_flow_fca_live on flow.fca (created_at) where deleted_at is null;
