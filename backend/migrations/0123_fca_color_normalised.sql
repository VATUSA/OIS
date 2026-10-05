-- @formatter:off
-- VATUSA/OIS#698: an FCA's colour is now validated on every write. It must be lowercase `#rrggbb`, the
-- only shape the map parses, and clear a contrast floor against the dark ground. Existing rows are
-- brought into that shape here, so that nothing stored earlier draws grey on the map while its list
-- chip shows the raw value.
--
-- The default moves from `#f59e0b`, which was no token, to `#efc14d` (`--series-3`, Amber, dark theme).
-- No CHECK constraint: the API is the guard, and a constraint could refuse a legacy row at startup.

alter table flow.fca alter column color set default '#efc14d';

update flow.fca set color = lower(trim(color)) where color <> lower(trim(color));

update flow.fca set color = '#efc14d' where color !~ '^#[0-9a-f]{6}$';
