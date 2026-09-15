-- @formatter:off
-- #278: taxiways are pavement polygons, not centerlines. `points` (an ordered [lat, lon] polyline)
-- becomes `rings` — the same shape as flow.airport_ramp_area.rings (array of rings of [lat, lon]),
-- so taxiways store, edit, and render as filled areas like ramps.
--
-- FAA rows already held a closed pavement outline in `points` (the importer's documented
-- outline-as-centerline mismatch), so they move over as that single ring. Every other row is an
-- open line that can't become a polygon without inventing a width, so it's deleted: 0067's KDCA
-- OSM seed, manual/CRC-drawn centerlines, and an FAA row a facility already re-drew as a line (a
-- "Re-pull FAA" restores its outline). The FAA seed covers KDCA; staff redraw by outline.

alter table flow.airport_taxiway add column if not exists rings jsonb;

update flow.airport_taxiway set rings = jsonb_build_array(points)
 where source = 'faa' and jsonb_array_length(points) >= 4 and points -> 0 = points -> -1;

delete from flow.airport_taxiway where rings is null;

alter table flow.airport_taxiway alter column rings set not null;

alter table flow.airport_taxiway drop column points;
