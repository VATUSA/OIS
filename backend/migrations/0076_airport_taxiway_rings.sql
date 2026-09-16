-- @formatter:off
-- #278: taxiways are pavement polygons, not centerlines. `points` (an ordered [lat, lon] polyline)
-- becomes `rings` — the same shape as flow.airport_ramp_area.rings (array of rings of [lat, lon]),
-- so taxiways store, edit, and render as filled areas like ramps.
--
-- FAA rows already held a closed pavement outline in `points` (the importer's documented
-- outline-as-centerline mismatch), so they move over as that single ring. Every other row is an
-- open line that can't become a polygon without inventing a width, so it's deleted: 0067's KDCA
-- OSM seed, manual/CRC-drawn centerlines, and any FAA row a facility re-drew as a line.
--
-- This deletion is permanent and `points` is dropped with it, so the old geometry can't be inspected
-- or re-derived afterwards. Only `source = 'faa'` rows at the 185 ICAOs in the bundled extract come
-- back, via "Re-pull FAA" (`repos::faa_surface_seed::seed_for_icao`, which returns NotFound outside
-- the extract and never touches manual/CRC rows). Everything else — every hand-drawn taxiway, and
-- every row at an airport outside the extract — is gone, and staff redraw it as an outline.

alter table flow.airport_taxiway add column if not exists rings jsonb;

update flow.airport_taxiway set rings = jsonb_build_array(points)
 where source = 'faa' and jsonb_array_length(points) >= 4 and points -> 0 = points -> -1;

delete from flow.airport_taxiway where rings is null;

alter table flow.airport_taxiway alter column rings set not null;

alter table flow.airport_taxiway drop column points;
