-- Engine-family aircraft profiles (#818): `flow.aircraft_profile` may hold `kind = 'family'` rows,
-- keyed `jet`, `turboprop` or `piston`. `ProfileTable::resolve` matches them after the exact type
-- and before the wake class. No rows are seeded; staff add them on
-- /admin/planning/aircraft-profiles.
--
-- 0059 declared the kind check inline, so Postgres named it. Drop whichever check constrains
-- `kind`, by the name pg_constraint holds, and add the replacement under an explicit name.

do $$
declare
    c text;
begin
    for c in
        select con.conname
          from pg_constraint con
          join pg_attribute att
            on att.attrelid = con.conrelid
           and att.attnum = any (con.conkey)
         where con.conrelid = 'flow.aircraft_profile'::regclass
           and con.contype = 'c'
           and att.attname = 'kind'
    loop
        execute format('alter table flow.aircraft_profile drop constraint %I', c);
    end loop;
end $$;

alter table flow.aircraft_profile
    add constraint aircraft_profile_kind_check
    check (kind in ('type', 'family', 'wake', 'default'));
