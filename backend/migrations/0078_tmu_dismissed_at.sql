-- #304: deleting a published TMI / ground stop / GDP keeps the row for historical replay but must
-- still take it off the TMU lists. `dismissed_at` marks rows the user removed; the list queries skip
-- them, while the replay (`*_at`) queries and history retention ignore it.
alter table tmu.tmis         add column if not exists dismissed_at timestamptz;
alter table tmu.ground_stops add column if not exists dismissed_at timestamptz;
alter table tmu.gdp          add column if not exists dismissed_at timestamptz;
