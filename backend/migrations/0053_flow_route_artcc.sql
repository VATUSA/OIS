-- Routes become per-ARTCC: a route drawn on a facility map belongs to that ARTCC, and the facility
-- map shows only its own routes plus the global (unassigned) ones. NULL artcc = global (visible on
-- every facility map + the national flow map); editing a global route needs national flow.route.update.
alter table flow.route add column if not exists artcc text;
create index if not exists idx_flow_route_artcc on flow.route(artcc);
