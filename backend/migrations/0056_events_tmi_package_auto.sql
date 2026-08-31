-- @formatter:off
-- Per-package auto-publish: like event FCAs, a TMI package can be flagged to activate automatically
-- 30 min before the event starts (and auto-archive when it ends), instead of a manual Activate click.

alter table events.tmi_package add column if not exists auto_publish boolean not null default false;
