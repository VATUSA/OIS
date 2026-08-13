-- @formatter:off
-- Helper for the cleanup job: resolve a ground stop's bare HHMM `until` (Zulu) to an
-- absolute instant — the first occurrence of that clock time at or after it was created.
-- Returns null for "until further notice" (no/invalid until).

create or replace function tmu.ground_stop_until_ts(p_created timestamptz, p_until text)
returns timestamptz
language sql
stable
as $$
  select case
    when p_until is null or p_until !~ '^[0-9]{4}$' then null
    else (
      date_trunc('day', p_created at time zone 'UTC')
      + make_interval(hours => left(p_until, 2)::int, mins => right(p_until, 2)::int)
      + case
          when date_trunc('day', p_created at time zone 'UTC')
               + make_interval(hours => left(p_until, 2)::int, mins => right(p_until, 2)::int)
               < (p_created at time zone 'UTC')
          then interval '1 day'
          else interval '0 day'
        end
    ) at time zone 'UTC'
  end
$$;
