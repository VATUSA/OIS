-- @formatter:off
-- Make tmu.gdp_end_ts deterministic: when two start occurrences are exactly equidistant from
-- the anchor (anchor ~12h from the window boundary), `order by abs(...)` alone leaves the day
-- ambiguous, which could expire a not-yet-run GDP a day early. Break the tie toward the LATER
-- occurrence so the resolved window (and thus expiry) is never earlier than intended.

create or replace function tmu.gdp_end_ts(p_anchor timestamptz, p_start text, p_end text)
returns timestamptz
language plpgsql
stable
as $$
declare
  au        timestamp;
  so        interval;
  eo        interval;
  base      date;
  start_ts  timestamp;
  end_ts    timestamp;
begin
  if p_start !~ '^[0-9]{4}$' or p_end !~ '^[0-9]{4}$' then
    return null;
  end if;
  au := p_anchor at time zone 'UTC';
  so := make_interval(hours => left(p_start, 2)::int, mins => right(p_start, 2)::int);
  eo := make_interval(hours => left(p_end,   2)::int, mins => right(p_end,   2)::int);
  base := date_trunc('day', au)::date;

  -- Start occurrence closest to the anchor; on a tie, the later one (so we never expire early).
  select cand into start_ts
  from unnest(array[(base - 1) + so, base + so, (base + 1) + so]) as cand
  order by abs(extract(epoch from (cand - au))), cand desc
  limit 1;

  -- End on the start's day, rolling to the next day when it isn't after the start.
  end_ts := date_trunc('day', start_ts) + eo;
  if end_ts <= start_ts then
    end_ts := end_ts + interval '1 day';
  end if;

  return end_ts at time zone 'UTC';
end
$$;
