-- @formatter:off
-- VATUSA/OIS#814: remove the learned taxi model. Its observations (0070, 0075) fed a per-gate,
-- type and runway ground allowance that every ground ETA added, so taxi time sat inside release
-- math. A ground flight with no scheduler time now takes its filed departure time + 10 minutes
-- (feed/predict.rs), and nothing reads this table.
--
-- The rows are dropped for good and cannot be recovered. They hold no personal data (no callsign
-- or CID). The table's indexes and its gate_id foreign key to flow.airport_gate go with it; nothing
-- else references flow.airport_gate.

drop table if exists stats.taxi_observation;
