# Discord integration

## Problem

One bot serving all Discord side effects: auto event threads + staff pings, TMI/ADV embeds, ACE request embeds with
claim buttons, and VATSIM↔Discord account linking.

## Architecture

Outbound queue + REST (locked decision). The bot owns no data.

- Backend inserts rows into `integration.outbound_jobs` (`job_type`, JSON `payload`,
  `status`, `attempt_count`, `next_attempt_at`).
- Bot polls the backend for pending jobs, performs the Discord action, acks success/failure.
- User interactions (claim button, slash commands) → bot calls the backend as a service account; backend applies the
  change and may enqueue follow-up jobs.

Config (guild id, channel/role/category maps) lives in `integration` tables, edited via
`discord.config.{read,update}`, mirroring osmium's `DiscordConfig`/channels/roles model.

## Job types *(draft)*

- `event_thread_create` — create thread/forum post + ping staff roles.
- `tmi_publish` / `adv_publish` — post an embed.
- `ace_request_post` — post request embed with claim button.
- `ace_request_notify` — DM/ping the EC on claim.

## Permissions

- `discord.config.{read,update}` — manage the guild/channel/role mapping.
- Feature-side publish gates live in their own domains (`events.discord.publish`,
  `tmu.tmi.publish`, `ace.requests.*`).

## Interactions the bot calls back for

- Claim button on an ACE request → `POST /ace/requests/{id}/claim` as the service account on behalf of the linked user.
- Slash commands (TBD): request ACE support, look up an event, etc.

## Open questions

- Poll interval vs. push (webhook/gateway) for job delivery.
- Account-linking flow: OAuth from the site vs. a bot command.
- Forums vs. threads for DCC (audit noted this as a consideration).
