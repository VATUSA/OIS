-- @formatter:off
-- #277: the taxi observer now measures pushback, start-up, and taxi-out as separate phases from
-- position-delta movement bursts instead of a single 7 kt crossing. Adds the new start-up phase
-- (push stop -> taxi start), nullable like pushback_sec: a no-tug departure or one first seen
-- already moving has no measurable push or start-up.
--
-- Existing rows are deleted, not backfilled: their pushback_sec measured parked -> first 7 kt roll
-- (gate dwell, not the push) and their taxi_sec started at that same crossing, so mixing them with
-- new-model rows would skew every learned median. The estimator falls back to its defaults until
-- fresh observations accumulate.

delete from stats.taxi_observation;

alter table stats.taxi_observation add column if not exists startup_sec integer;
