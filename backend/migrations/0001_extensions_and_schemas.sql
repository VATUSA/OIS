create
extension if not exists pgcrypto;
create
extension if not exists citext;

-- Per-domain schemas (mirrors osmium; OIS adds tmu / ace / flow).
create schema if not exists platform;
create schema if not exists identity;
create schema if not exists access;
create schema if not exists org;
create schema if not exists events;
create schema if not exists tmu;
create schema if not exists ace;
create schema if not exists flow;
create schema if not exists integration;
create schema if not exists stats;
create schema if not exists media;
create schema if not exists web;

create
or replace function platform.touch_updated_at()
returns trigger
language plpgsql
as $$
begin
    new.updated_at
= now();
return new;
end;
$$;
