-- Link an advisory to the program that generated it (#508).
--
-- A GDP advisory and its tmu.gdp row are the same event (#461's decision), so the advisory is now
-- generated from the program at publish rather than hand-authored. Revising a published program
-- changes its delay figures, and #461 says advisories are cancelled and reissued rather than
-- rewritten — which means the revise path has to find the program's live advisory. That is what these
-- columns are for.
--
-- Numbered 0088, not 0087: #431 (PR #515) already adds 0087_airport_gate_xplane.sql, and two
-- migrations sharing a version is an sqlx conflict.

alter table tmu.advisories
    add column if not exists gdp_id text references tmu.gdp(id) on delete set null;
alter table tmu.advisories
    add column if not exists ground_stop_id text references tmu.ground_stops(id) on delete set null;

comment on column tmu.advisories.gdp_id is
    'The GDP this advisory was generated from (#508); null for a hand-authored advisory.';
comment on column tmu.advisories.ground_stop_id is
    'The ground stop this advisory was generated from (#508); null for a hand-authored advisory.';

-- `on delete set null` rather than cascade: an issued advisory is a published artefact and outlives
-- the program it describes. Deleting a never-published draft program must not erase documents, and
-- repos::tmu::delete_or_retain already retains anything that was published.

-- Partial indexes: the revise path looks a program's live advisory up by program id, and the vast
-- majority of rows are hand-authored with both columns null.
create index if not exists idx_tmu_advisories_gdp
    on tmu.advisories(gdp_id) where gdp_id is not null;
create index if not exists idx_tmu_advisories_ground_stop
    on tmu.advisories(ground_stop_id) where ground_stop_id is not null;
