-- Keep more of what X-Plane gives us per stand (VATUSA/OIS#541).
--
-- The importer parsed heading and then threw it away, and skipped row `1301` and row `1300`'s
-- aircraft-class field entirely, because there was nowhere to put any of it. These are those places.
--
-- First-class columns rather than one jsonb blob: the shape is already known and enumerable --
-- `1301 B airline aal dal`, with classes pipe-delimited from a fixed vocabulary
-- (heavy|jets|turboprops|props) -- so a blob would hide structure we have rather than defer
-- modelling it, and make "which stands take a heavy" awkward to ask.
--
-- All nullable, and nothing backfills them: rows from the other sources (manual, osm, crc, faa) have
-- no X-Plane datum to carry, exactly as `kind` is already null for those.

alter table flow.airport_gate
    -- Degrees true, normalised into [0, 360) by the importer. The Gateway serves it unnormalised
    -- (-510.9 appears in KDCA's first row), so the normalisation is the importer's job, not a
    -- consumer's.
    add column heading double precision
        check (heading is null or (heading >= 0 and heading < 360)),
    -- ICAO aerodrome reference code letter from row `1301` -- A..F, widest aircraft the stand takes.
    add column size_code text
        check (size_code is null or size_code ~ '^[A-F]$'),
    -- Row `1301`'s operation type, e.g. `airline`, `cargo`, `general_aviation`. Not constrained: it
    -- is community-contributed free text and a new value should not fail an import.
    add column operation_type text,
    -- Row `1300`'s aircraft classes, split on `|`, e.g. {heavy,jets}.
    add column aircraft_classes text[],
    -- Row `1301`'s airline codes, e.g. {aal,dal}. Lowercase as the source serves them.
    add column airline_codes text[];

-- `size_code` is the field most likely to be queried on (the taxi-observation refinement that
-- already names `kind` as its hook would use it the same way), and only X-Plane rows have one.
create index airport_gate_size_code_idx on flow.airport_gate (icao, size_code)
    where size_code is not null;
