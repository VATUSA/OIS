-- Website-configured event-thread message body, replacing the hard-coded Rust format! block in
-- discord/src/jobs/thread.rs. Singleton table (one 'default' row); seeded with the current
-- hard-coded text (placeholders substituted by the bot at render time) so output is unchanged
-- until an admin edits it. Reuses discord.config.read/update — same admin surface.

create table if not exists integration.event_thread_template (
    id         text primary key default 'default',
    body       text not null,
    updated_at timestamptz not null default now()
);

create trigger trg_integration_event_thread_template_updated_at
before update on integration.event_thread_template
for each row execute function platform.touch_updated_at();

insert into integration.event_thread_template (id, body) values (
    'default',
    $$**{{title}} | Planning Thread**
{{title}} is on {{date_line}}

Review the following for your facility:
- TMU/TMI package
- Staffing
- Configs and AAR

{{facility_lines}}
Attempt to coordinate as many plans (initiatives, reroutes, etc.) in a timely manner, and fill out all appropriate areas of the staffing data.
───────────────────────────
{{ntmo_ping}} please react with your availability to NOM for this event. {{dcc_ping}} please react with your availability to shadow this event.

🟢 = Available
🟡 = Partially available/unsure
🔴 = Unavailable
───────────────────────────$$
) on conflict (id) do nothing;
