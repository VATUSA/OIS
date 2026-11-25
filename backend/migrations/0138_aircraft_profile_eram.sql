-- @formatter:off
-- A type with an ERAM record flies it unless flow.aircraft_profile holds a type row for it (#985). The
-- rows 0059 seeded for types ERAM has would go on winning, so the ones still holding exactly the numbers
-- 0059 gave them are removed; a row with any number changed is an operator's and stays. PA28 has no ERAM
-- record and keeps its row.
--
-- The deleted rows are not recoverable from the database. A row wanted back is re-entered on the
-- aircraft-profiles page. Each replica's profile cache picks the change up at its next refresh. A replica still on
-- the release before this one has no ERAM tables, so until it is replaced it flies these types on
-- their family, wake or default rows.
delete from flow.aircraft_profile p
using (values
    ('C172',  75::float8,  90::float8, null::float8,  500::float8,  500::float8, 110::float8, null::float8, 14000::float8, null::float8, 110::float8,  90::float8,  500::float8),
    ('C208', 100, 120, null,  900,  700, 170, null, 25000, null, 140, 110,  800),
    ('CRJ7', 250, 290, 0.78, 2000, 1500, 440, 0.78, 41000, 0.78, 290, 250, 1800),
    ('E75L', 250, 290, 0.78, 2200, 1600, 450, 0.78, 41000, 0.78, 290, 250, 1800),
    ('B738', 250, 290, 0.78, 2500, 1800, 450, 0.78, 41000, 0.78, 290, 250, 1800),
    ('A320', 250, 290, 0.78, 2200, 1800, 450, 0.78, 39000, 0.78, 290, 250, 1800),
    ('B763', 250, 300, 0.80, 2200, 1600, 460, 0.80, 41000, 0.80, 300, 250, 1900),
    ('B77W', 250, 310, 0.84, 2500, 1800, 490, 0.84, 43000, 0.84, 300, 250, 2000),
    ('B789', 250, 310, 0.85, 2600, 1900, 490, 0.85, 43000, 0.85, 300, 250, 2000),
    ('A359', 250, 310, 0.85, 2600, 1900, 490, 0.85, 43000, 0.85, 300, 250, 2000)
) as seed (key, climb_ias_lo, climb_ias_hi, climb_mach, climb_fpm_lo, climb_fpm_hi,
           cruise_tas, cruise_mach, service_ceiling_ft,
           desc_mach, desc_ias_hi, desc_ias_lo, desc_fpm)
where p.kind = 'type'
  and p.key = seed.key
  and p.climb_ias_lo is not distinct from seed.climb_ias_lo
  and p.climb_ias_hi is not distinct from seed.climb_ias_hi
  and p.climb_mach is not distinct from seed.climb_mach
  and p.climb_fpm_lo is not distinct from seed.climb_fpm_lo
  and p.climb_fpm_hi is not distinct from seed.climb_fpm_hi
  and p.cruise_tas is not distinct from seed.cruise_tas
  and p.cruise_mach is not distinct from seed.cruise_mach
  and p.service_ceiling_ft is not distinct from seed.service_ceiling_ft
  and p.desc_mach is not distinct from seed.desc_mach
  and p.desc_ias_hi is not distinct from seed.desc_ias_hi
  and p.desc_ias_lo is not distinct from seed.desc_ias_lo
  and p.desc_fpm is not distinct from seed.desc_fpm;
