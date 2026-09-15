-- Add 'faa' as a source for airport surface geometry (#230/#231): the FAA Aerodrome Mapping
-- extract seeds flow.airport_ramp_area / flow.airport_taxiway with source='faa' rows. Extending
-- the gate table's check too even though this issue writes no gate rows (FAA AM has no gate
-- layer, per #230) — keeps all three flow.airport_* tables' source domains in sync.

alter table flow.airport_gate
    drop constraint if exists airport_gate_source_check;
alter table flow.airport_gate
    add constraint airport_gate_source_check
    check (source in ('manual', 'osm', 'crc', 'faa'));

alter table flow.airport_ramp_area
    drop constraint if exists airport_ramp_area_source_check;
alter table flow.airport_ramp_area
    add constraint airport_ramp_area_source_check
    check (source in ('manual', 'osm', 'crc', 'faa'));

alter table flow.airport_taxiway
    drop constraint if exists airport_taxiway_source_check;
alter table flow.airport_taxiway
    add constraint airport_taxiway_source_check
    check (source in ('manual', 'osm', 'crc', 'faa'));
