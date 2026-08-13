-- @formatter:off
-- Per-event airport rates: planned AAR (arrival) / ADR (departure) per airport. Set by
-- facility staff (events.rate.update, scoped to the airport's owning ARTCC) or the
-- events team. On activation these seed the live tmu.program rates (TMI-packages pass).

create table if not exists events.airport_rate (
    event_id bigint not null references events.event(id) on delete cascade,
    icao text not null,
    aar int not null default 0 check (aar between 0 and 200),
    adr int not null default 0 check (adr between 0 and 200),
    -- owning ARTCC resolved when the rate was set (for display + scope audit); '' if unknown
    artcc text not null default '',
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (event_id, icao)
);

create trigger trg_events_airport_rate_updated_at
before update on events.airport_rate
for each row execute function platform.touch_updated_at();
