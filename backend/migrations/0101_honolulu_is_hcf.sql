-- Honolulu is `HCF`, end to end (VATUSA/OIS#556).
--
-- OIS used two ids for Honolulu. `org.facilities`, VATUSA, grants and TMU scope all say `HCF`; the
-- bundled boundary assets, `center_artcc` and the feed's VATSpy FIR mapping (`PHZH`) said `ZHN`. The code
-- now says `HCF` everywhere except `feed/neighbors.rs`, which aliases a genuinely third-party dataset.
--
-- This repoints rows already stored under `ZHN`. `ZHN` is not, and never was, a row in `org.facilities`,
-- so a stored `'ZHN'` can only ever mean Honolulu, and the rename is unambiguous. Without this, deployed
-- rows would stay stranded under an id that no grant or scope reaches — the same failure 0041 avoided
-- for a renamed Discord channel.
--
-- ## Why the columns are found here, not listed
--
-- Several unconstrained columns hold an ARTCC id — some typed by an admin, some the *owning ARTCC* of an
-- airport derived from the very feed mapping this change corrects (`flow.fca.artcc`,
-- `flow.airport_config.artcc`, …), and `stats.controller_session.facility`, written by stats collection.
-- A hand-written list was wrong twice while this was being written. So the migration asks
-- `information_schema` at deploy time, against the schema actually present.
--
-- ## Why it cannot fail startup
--
-- Seven of these columns sit in a unique or primary key (e.g. `tmu.advisories` (facility, issued_day,
-- number)). If an `HCF` twin already exists, repointing the `ZHN` row would violate it, and a failed
-- migration half-applies the deploy. So each column is updated in one statement, and on
-- `unique_violation` falls back to row by row, leaving **only** a genuinely colliding row as `ZHN`.
-- It is inert, and it is duplicated by its `HCF` twin anyway.
--
-- `org` is skipped: it is the reference the other columns point at, and it has no `ZHN` row.

do $$
declare
    col record;
    r record;
begin
    for col in
        select c.table_schema, c.table_name, c.column_name
          from information_schema.columns c
          join information_schema.tables t
            on t.table_schema = c.table_schema
           and t.table_name = c.table_name
           and t.table_type = 'BASE TABLE'
         where c.column_name in ('artcc', 'artcc_id', 'facility', 'facility_id')
           and c.data_type in ('text', 'character varying')
           and c.table_schema not in ('org', 'pg_catalog', 'information_schema')
    loop
        begin
            execute format('update %I.%I set %I = %L where %I = %L',
                           col.table_schema, col.table_name, col.column_name, 'HCF',
                           col.column_name, 'ZHN');
        exception when unique_violation then
            for r in execute format('select ctid from %I.%I where %I = %L',
                                    col.table_schema, col.table_name, col.column_name, 'ZHN')
            loop
                begin
                    execute format('update %I.%I set %I = %L where ctid = %L',
                                   col.table_schema, col.table_name, col.column_name, 'HCF',
                                   r.ctid);
                exception when unique_violation then
                    null; -- an HCF twin exists; this row stays ZHN, duplicated by it
                end;
            end loop;
        end;
    end loop;
end $$;
