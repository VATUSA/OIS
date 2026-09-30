-- @formatter:off
-- Advisories — the vATCSCC ADVZY document entity (#457, sub-issue of #437).
--
-- `0008_tmu.sql` said "NTML entries + advisories follow the same pattern in later migrations" and
-- seeded `tmu.adv.*`; this is that table, seven years of "later" on. Shaped like `tmu.tmis`: the same
-- draft->published lifecycle, the same `structured`/`decoded` split a raw-typed document leaves null,
-- and the same `touch_updated_at` trigger.
--
-- What an advisory has that a TMI does not is a **number**. `vATCSCC ADVZY 002` is a sequence per
-- issuing facility per Zulu day, and it is part of the document's identity — so it lives here rather
-- than being computed when the document is rendered.
--
-- `facility` is deliberately not a foreign key to `org.facilities`: an advisory can be issued by
-- something that is not an ARTCC in that table (the DCC most obviously), and a missing row must not
-- stop an advisory being issued. It is `not null` because you cannot number per facility without one.
--
-- 0085 rather than 0081-0084: those are claimed by the unmerged #433, #432, #436 and #444 branches.

create table if not exists tmu.advisories (
    id           text not null primary key default gen_random_uuid()::text,
    facility     text not null,
    -- The Zulu date the number was allocated on. Stored rather than derived from `created_at` so the
    -- sequence a reader sees can never disagree with the one the allocator used, whatever the
    -- server's timezone does.
    issued_day   date not null,
    number       integer not null,
    -- Which ADVZY document this is: reroute, ground delay program, ground stop. Left open here —
    -- #458 and #461 add the types and their field sets.
    kind         text not null,
    -- The rendered document, and the fields it came from when it was built rather than typed.
    body         text not null,
    structured   jsonb,
    decoded      text,
    status       text not null default 'draft'
        check (status in ('draft', 'published', 'cancelled')),
    created_by   text references identity.users(id) on delete set null,
    published_by text references identity.users(id) on delete set null,
    published_at timestamptz,
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),

    -- What actually guarantees two advisories never share a number, however the allocator is
    -- written. The allocator serialises so this is not the error path in practice — but a race that
    -- slipped past it must fail the insert rather than duplicate a document's identity.
    constraint uq_tmu_advisories_number unique (facility, issued_day, number)
);

create index if not exists idx_tmu_advisories_status on tmu.advisories (status);
create index if not exists idx_tmu_advisories_facility_day
    on tmu.advisories (facility, issued_day desc);

create trigger trg_tmu_advisories_updated_at
before update on tmu.advisories
for each row execute function platform.touch_updated_at();
