# Discord integration

## Problem

One Discord bot serves every Discord-facing side effect in OIS so no feature talks to Discord directly. It replaces
the ad-hoc Discord code scattered across cobalt/webapps and covers four concrete jobs from the stakeholder audit:

- Auto-create a DCC (Discord Coordination Channel) thread and ping required staff when an event is published
  (see [events-workflow.md](events-workflow.md)).
- Post embeds for published TMIs and ADVs (see [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)).
- Post ACE support requests as embeds with a **claim** button and notify the EC on claim
  (see [ace-support.md](ace-support.md)).
- Link a VATSIM identity to a Discord user so interactions and pings resolve to the right person.

The bot is a separate Rust binary (serenity + poise). It **owns no data**: every fact it needs lives in Postgres behind
the versioned API (`/api/v1`), and every action it takes is either dispatched to it as a job or written back through the
API. This keeps Discord state fully reconstructable from the backend and lets the bot be restarted or replaced without
data loss.

## Architecture

Locked decision: **outbound job queue + REST**. There is no direct backend→Discord call path.

**Outbound (backend → Discord).** A feature that needs a Discord side effect inserts a row into
`integration.outbound_jobs` inside the same transaction as its own state change (e.g. an event moving to published, a TMI
moving to `published`). The row carries:

| column | meaning |
| --- | --- |
| `job_type` | discriminator selecting the handler (see [Job types](#job-types)) |
| `payload` (jsonb) | everything the handler needs: resolved channel/role logical names, embed fields, subject ids |
| `subject_type` / `subject_id` | back-reference to the originating row (e.g. `event` / `<event_id>`) for idempotency and audit |
| `status` | `pending` → `in_progress` → `succeeded` / `failed` |
| `attempt_count` | incremented per delivery attempt |
| `next_attempt_at` | earliest time the job is eligible; set into the future for backoff |
| `last_attempt_at`, `error` | last attempt time and last failure reason |

Job lifecycle:

1. **Enqueue** — feature handler inserts a `pending` job with `next_attempt_at = now()`.
2. **Lease** — the bot asks the backend for due pending jobs (`status = pending AND next_attempt_at <= now()`); the
   backend marks them `in_progress` and hands them over. Leasing is claim-and-lock so concurrent bot instances don't
   double-deliver.
3. **Perform** — the bot executes the Discord action (create thread, post embed, edit embed, DM/ping).
4. **Ack** — the bot calls back to mark the job `succeeded` (recording any Discord ids the backend must remember, e.g.
   the created message/thread id) or `failed` (with `error`). On failure the backend bumps `attempt_count` and pushes
   `next_attempt_at` out by a backoff; past a max attempt count the job is left `failed` for operator review.

**Interaction callback (Discord → backend).** For user-initiated interactions (button clicks, slash commands) the bot
does **not** mutate anything itself. It calls back into `/api/v1` as a **service account** (bearer token, matched by
hash against an active credential — osmium's service-account model). The backend applies the
change under the acting user's identity (resolved via the account link), enforces the feature permission and any
data-state rule, and **may enqueue follow-up outbound jobs** (e.g. a claim triggers an embed edit + an EC notification).
The bot therefore never holds authority — it is a transport for the queue in one direction and a thin proxy for
interactions in the other.

Discord ids the backend needs to remember (created thread ids, posted message ids, the VATSIM↔Discord mapping) are
persisted in `integration` tables — notably `external_sync_mappings` (`system_code = 'discord'`) — not in the bot.

## Job types

Each `job_type` is one outbound handler. `trigger` is the backend event that enqueues it; `effect` is what the bot does.

| `job_type` | trigger | effect |
| --- | --- | --- |
| `event_thread_create` | event published with `events.discord.publish` | create the DCC thread (or forum post) under the configured category/channel and ping the required staff roles; ack with the created thread id |
| `tmi_publish` | TMI moves to `published` (`tmu.tmi.publish`) | post an embed of the plain-language TMI to the configured TMU channel |
| `adv_publish` | advisory published (`tmu.adv.*`) | post an embed of the advisory to the configured TMU channel |
| `ace_request_post` | new ACE request created (`ace.requests.create`) | post an embed to the configured ACE-requests channel with a **claim** button; ack with the posted message id |
| `ace_request_notify` | request claimed, via button or site (`ace.requests.claim`) | edit the original request embed to show the claimer and notify the requesting EC (DM/ping) |

Notes:
- Channels, roles, and the category for event threads are referenced by **logical name** in the payload and resolved
  against the `integration` config tables at enqueue time, so the bot receives concrete ids.
- `ace_request_notify` depends on the message id captured when `ace_request_post` acked — the backend stores that id
  and includes it in the follow-up job's payload.
- Additional job types (announcements, scheduled Discord events, role sync) are out of scope for the first cut but fit
  the same table without schema change.

## Config & account linking

**Config.** Guild id, channel map, role map, and category map live in `integration` tables and are edited via
`discord.config.{read,update}`. This mirrors osmium's model:

| table | holds |
| --- | --- |
| `integration.discord_configs` | one row per guild: `name`, `guild_id` |
| `integration.discord_channels` | logical `name` → Discord `channel_id`, scoped to a config |
| `integration.discord_roles` | logical `name` → Discord `role_id` (the staff roles pinged on event publish) |
| `integration.discord_categories` | logical `name` → Discord `category_id` (parent for DCC threads/forum) |
| `integration.discord_config_facilities` | which ARTCC(s) a guild's config serves — `(config_id, artcc_id)`, many-to-many |

Features reference channels/roles/categories by **logical name** (e.g. `dcc`, `tmu`, `aceteam-requests`), never by raw
snowflake, so retargeting a channel is a config edit and touches no feature code. To spare operators from pasting
snowflakes, the config UI can populate dropdowns from a live guild snapshot proxied through the bot (guilds, channels,
categories, roles).

**Multi-guild name collisions.** Two guilds can each configure the same logical name (e.g. both defining
`aceteam-requests`) — `discord_config_facilities` is how `channel_id`/`role_id` (`backend/src/repos/integration.rs`)
pick the right one: a caller that knows the relevant facility (an ACE request's ARTCC, an event's host) passes it, and
the guild whose facilities include that ARTCC wins the name over any other guild defining it. A caller with no
facility to pass, or a facility no guild claims, falls back to whichever guild was configured first — the same
behavior as before facility-scoping existed. Not every call site is facility-scoped: `handlers::tmu::publish_tmi`'s
TMU channel and the generic `ntmo`/`dcc-trainee` event-thread roles are left unscoped since they don't have a single
unambiguous owning facility.

**Account linking.** Linking ties a VATSIM identity to a Discord user; the mapping is stored in
`integration.external_sync_mappings` (`system_code = 'discord'`, `entity_type = 'user'`, `local_id = <VATSIM cid>`,
`external_id = <Discord user id>`). First-cut flow mirrors osmium's site-driven OAuth:

1. Signed-in user starts linking from the OIS site (`POST /api/v1/me/discord/link/start` with a `return_url`); the
   backend creates a stored OAuth `state` record and returns the Discord `auth_url`.
2. The browser goes to Discord; Discord redirects to the backend's own callback with `?code&state`.
3. The backend exchanges the code server-side, writes the mapping (idempotent per user — re-linking updates rather than
   duplicates), and 302s back to `return_url` with a success/error flag.
4. Unlink removes the mapping.

The link lets the backend resolve a Discord interaction (e.g. a claim click) to the OIS user whose permissions then
gate the action, and lets outbound jobs ping/DM the correct Discord user.

## Permissions

Path-based `segments.action`. Only the Discord **mapping** is gated in this domain:

- `discord.config.read` — view the guild/channel/role/category mapping.
- `discord.config.update` — edit it.

**Feature-side publish gates live in their own domains**, not here — the ability to cause a Discord side effect is a
property of the feature:

| gate | domain | governs |
| --- | --- | --- |
| `events.discord.publish` | events | enqueue `event_thread_create` |
| `tmu.tmi.publish` | TMU | enqueue `tmi_publish` |
| `tmu.adv.*` | TMU | enqueue `adv_publish` |
| `ace.requests.create` | ACE | enqueue `ace_request_post` |
| `ace.requests.claim` | ACE | claim (button or site) → `ace_request_notify` |

The bot's own callbacks authenticate as a service account holding least-privilege roles; the backend still evaluates the
above feature permissions against the **linked user's** identity, not the service account's.

## Interactions the bot calls back for

- **ACE claim button** → the bot POSTs `POST /api/v1/ace/requests/{id}/claim` as the service account, on behalf of the
  linked user who clicked. The backend checks `ace.requests.claim`, checks the request is still open (data-dependent on
  top of the permission), records the claimer, and enqueues `ace_request_notify` (edit embed + notify EC). If the user
  has no account link, the backend rejects and the bot surfaces a prompt to link first.
- **Slash commands (TBD)** — candidates: request ACE support, look up an upcoming event. Each would follow the same
  pattern: bot collects input, POSTs to the relevant `/api/v1` endpoint as the service account on behalf of the linked
  user, backend applies and may enqueue follow-up jobs. Command surface and gating are not yet specified.

## Open questions

- **Job delivery: poll vs. push.** First cut is the bot polling the backend for due jobs. A push/gateway model (backend
  notifies the bot) would cut latency but adds a connection to manage — decision deferred.
- **Account linking entry point.** Site OAuth (above) vs. a bot slash command that starts the same flow. Site-first is
  assumed; a bot command could be added later over the same mapping.
- **DCC: threads vs. forums.** The audit flagged migrating the DCC from threads to forum posts. `event_thread_create`
  is written to cover either target so the choice can be made in config without reworking the job.
