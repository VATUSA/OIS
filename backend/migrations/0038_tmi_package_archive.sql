-- @formatter:off
-- TMI packages gain a post-event lifecycle: an activated package can be *deactivated*, which cancels
-- the live TMU rows it created and archives the package as a read-only record of what was active.
-- To cancel exactly what activation created, each item records a `live_ref` to its materialized row
-- (the tmu id for restrictions/ground stops; the airport ICAO for programs, which are keyed by ICAO).

alter table events.tmi_package
    drop constraint if exists tmi_package_status_check;
alter table events.tmi_package
    add constraint tmi_package_status_check
    check (status in ('draft', 'activated', 'archived'));

alter table events.tmi_package
    add column if not exists archived_at timestamptz;

alter table events.tmi_package_item
    add column if not exists live_ref text;
