-- @formatter:off
-- VATUSA/OIS#817: an FCA freeze horizon. An airborne flight whose ETA to the crossing is inside
-- `freeze_min` keeps the STA it was given when it entered; outside it is re-metered on every read.
-- 0 turns freezing off. The default, 30 min, is the low end of TBFM's ~30-40 min (MIT LL ATC-364 §2).
alter table flow.fca
    add column if not exists freeze_min int not null default 30 check (freeze_min between 0 and 120);

-- The STA each airborne flight was given on entering an FCA's freeze horizon. The held-STA pass
-- (`jobs::spawn_fca_hold_sync`) writes a row the first time it sees a flight inside the horizon
-- (`on conflict do nothing`, so every replica keeps the first STA stored) and deletes it once the
-- flight leaves the horizon by more than the margin, is released or leaves the feed; reads only read
-- it. Rows an hour past their STA are ignored, and deleted by the FCA's next sync that stores or
-- drops a row. An edit to a metering field, a manual reorder and a delete clear the FCA's rows in
-- the same statement, and the pass deletes the rows of every FCA it does not meter (disabled,
-- deleted, unpublished event FCA).
create table if not exists flow.fca_held_sta (
    fca_id   text not null references flow.fca(id) on delete cascade,
    callsign text not null,
    sta_ms   bigint not null,     -- frozen metered crossing time, epoch ms
    primary key (fca_id, callsign)
);
