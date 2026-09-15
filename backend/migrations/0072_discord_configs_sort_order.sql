-- A full-replace `upsert_config` save relies on `created_at`'s `default now()`, but Postgres's
-- `now()` returns the *transaction's* start time — every guild inserted in that one transaction
-- gets an identical timestamp, making the fallback tiebreak in `resolve_scoped_id` depend on
-- Postgres's unspecified same-timestamp row order (#203). `sort_order`, set explicitly from the
-- request array's position, replaces `created_at` as that tiebreak.

alter table integration.discord_configs add column sort_order integer not null default 0;
