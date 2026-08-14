-- @formatter:off
-- GDP departure scope: restrict a Ground Delay Program to flights departing from a set of
-- ARTCCs (space-separated center codes). Empty = every departure, like ground stops. Flights
-- outside the scope are exempt (not held on the ground).

alter table tmu.gdp
    add column if not exists scope text not null default '';
