# Discord integration

> **Status: built.** The outbound-job queue, the bot (`discord/`, serenity — no `poise`), and every
> job type below except `adv_publish` (never built — there's no separate advisory record; see
> [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)) are shipped. Account linking is **not** the
> self-service OAuth flow this doc originally described — see [Config & account linking](#config--account-linking)
> below for what's actually built. This doc is kept current; sections describing something not yet
> built say so explicitly.

## Problem

One Discord bot serves every Discord-facing side effect in OIS so no feature talks to Discord directly. It replaces
the ad-hoc Discord code scattered across cobalt/webapps and covers four concrete jobs from the stakeholder audit:

- Auto-create a DCC (Discord Coordination Channel) thread and ping required staff when an event is published
  (see [events-workflow.md](events-workflow.md)).
- Post published TMIs to Discord (see [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)) — advisories (ADV)
  were never implemented as a separate record, so only TMIs post today.
- Post ACE support requests as embeds with a **claim** button and notify the EC on claim
  (see [ace-support.md](ace-support.md)).
- Link a VATSIM identity to a Discord user so interactions and pings resolve to the right person.

The bot is a separate Rust binary (serenity). It **owns no data**: every fact it needs lives in Postgres behind
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

**Interaction callback (Discord → backend).** For user-initiated interactions (button clicks, select menus, modal
submits — see [Interactions the bot calls back for](#interactions-the-bot-calls-back-for)) the bot does **not** mutate anything itself. It calls back into `/api/v1` as a **service account** (bearer token, matched by
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
| `event_thread_create` | event published with `events.discord.publish` | create a public **thread** (not a forum post — that option was decided against, see below) under the configured channel from a configurable text template, and ping the required staff roles; ack with the created thread id |
| `tmi_publish` | TMI moves to `published` (`tmu.tmi.publish`) | post the raw restriction text (plus a requesting/providing header line and a **View structured** button) to the configured TMU channel |
| `ace_request_post` | new ACE request created (`ace.requests.create`) | post an embed to the configured ACE-requests channel with a **claim** button; ack with the posted message id |
| `ace_request_notify` | request claimed, via button or site (`ace.requests.claim`) | edit the original request embed to show the claimer and notify the requesting EC (DM/ping) |
| `ace_claim_dm` | a claim links to a Discord account, right after `ace_request_notify` | DM the claimer a summary of what they signed up for |
| `ace_claim_reminder_24h` / `ace_claim_reminder_6h` | a periodic backend scheduler finds a confirmed claim whose covered time is due in that window | DM the claimer a reminder; deduped per tier via a DB unique index so a rolling deploy can't double-send |
| `guild_snapshot` | the admin config page's "Refresh from Discord" button | re-pull every guild the bot is in (channels + roles) and push the snapshot the config editor's dropdowns read from |

There is no `adv_publish` job — advisories were never implemented as a separate first-class record
(see [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)); only TMIs post to Discord today.

Notes:
- Channels and roles are referenced by **logical name** in the payload and resolved against the
  `integration` config tables at enqueue time, so the bot receives concrete ids. (`discord_categories`
  is modeled in the schema but nothing currently enqueues against it — event threads are created
  directly under a channel, not a category.)
- `ace_request_notify` depends on the message id captured when `ace_request_post` acked — the backend stores that id
  and includes it in the follow-up job's payload.
- Additional job types (announcements, scheduled Discord events, role sync) are out of scope for the first cut but fit
  the same table without schema change.

## Config & account linking

**Config.** Guild id, channel map, and role map live in `integration` tables and are edited via
`discord.config.{read,update}`:

| table | holds |
| --- | --- |
| `integration.discord_configs` | one row per guild: `name`, `guild_id` |
| `integration.discord_channels` | logical `name` → Discord `channel_id`, scoped to a config (`unique(config_id, name)`) |
| `integration.discord_roles` | logical `name` → Discord `role_id`, scoped to a config (the staff roles pinged on event publish) |
| `integration.discord_categories` | logical `name` → Discord `category_id`, scoped to a config — modeled but currently unused (see [Job types](#job-types)) |
| `integration.discord_config_facilities` | which ARTCC(s) a guild's config serves — `(config_id, artcc_id)`, many-to-many |

Features reference channels/roles by **logical name** (e.g. `tmu-advisories`, `aceteam-requests`, `events`, or a
per-region name like `region-zdc`), never by raw snowflake, so retargeting a channel is a config edit and touches no
feature code. The config UI populates dropdowns from a live guild snapshot the bot pushes on connect and on-demand
(the `guild_snapshot` job).

**Multi-guild is supported for both guild *discovery* (`snapshot_and_push` iterates every guild the bot is in, with
no hard-coded guild id anywhere in `ois-discord` or the backend) and, since #194, the generic per-feature
channel/role names (`aceteam-requests`, `tmu-advisories`, `events`, `ntmo`, `dcc-trainee`)** — see **Multi-guild name
collisions** below for how `channel_id`/`role_id` resolve which guild wins when two define the same name.

**Walkthrough: adding a guild.** Invite the bot to the new guild, then in the admin config page
(`/admin/discord`, `discord.config.update`): click "Refresh from Discord" (enqueues `guild_snapshot`
so the new guild's channels/roles populate the dropdowns) → **Add guild**, pick it from the synced
guild list, map whichever logical channel/role names that guild needs, and set the ARTCC(s) it
serves in `facilities` if it shares a generic name with another guild → **Save**. No code change, no
redeploy, no migration.

**Multi-guild name collisions.** Two guilds can each configure the same logical name (e.g. both defining
`aceteam-requests`) — `discord_config_facilities` is how `channel_id`/`role_id` (`backend/src/repos/integration.rs`)
pick the right one: a caller that knows the relevant facility (an ACE request's ARTCC, an event's host) passes it, and
the guild whose facilities include that ARTCC wins the name over any other guild defining it. A caller with no
facility to pass, or a facility no guild claims, falls back to whichever guild was configured first — the same
behavior as before facility-scoping existed. Not every call site is facility-scoped: `handlers::tmu::publish_tmi`'s
TMU channel and the generic `ntmo`/`dcc-trainee` event-thread roles are left unscoped since they don't have a single
unambiguous owning facility.

**Account linking.** Linking ties a VATSIM identity to a Discord user; the mapping is stored in
`integration.external_sync_mappings` (`system_code = 'discord'`, `entity_type = 'user'`, `local_id = <OIS user id>`,
`external_id = <Discord user id>`). **This is not a self-service OAuth flow** (the original spec below described one;
none was built) — the mapping is populated read-only from VATUSA's own member data during the routine VATUSA roster
sync (`backend/src/repos/vatusa.rs`), which already carries each member's linked Discord id. There is no
`/me/discord/link/start`/OAuth-callback pair and no unlink action in OIS; the only OIS-side surface is a read-only
`GET /api/v1/me/discord` that reports whether the signed-in user has a synced link. Whatever this doc originally
proposed for the link flow itself never got built, because it turned out to be unnecessary — VATUSA already has the
data.

The link lets the backend resolve a Discord interaction (e.g. a claim click) to the OIS user whose permissions then
gate the action, and lets outbound jobs ping/DM the correct Discord user.

## Permissions

Path-based `segments.action`. Only the Discord **mapping** is gated in this domain:

- `discord.config.read` — view the guild/channel/role mapping.
- `discord.config.update` — edit it.

**Feature-side publish gates live in their own domains**, not here — the ability to cause a Discord side effect is a
property of the feature:

| gate | domain | governs |
| --- | --- | --- |
| `events.discord.publish` | events | enqueue `event_thread_create` |
| `tmu.tmi.publish` | TMU | enqueue `tmi_publish` |
| `ace.requests.create` | ACE | enqueue `ace_request_post` |
| `ace.requests.claim` | ACE | claim (button or site) → `ace_request_notify` (+ `ace_claim_dm` if the claimer is Discord-linked) |

The bot's own callbacks authenticate as a service account holding least-privilege roles; the backend still evaluates the
above feature permissions against the **linked user's** identity, not the service account's.

## Interactions the bot calls back for

All interactions are Discord message components (buttons, select menus, modals) — there are no slash commands.

- **ACE claim** → clicking **claim** on the request embed opens an ephemeral start/end time-picker (select menus,
  `custom_id` prefixes `aceS:`/`aceE:`); a **confirm** button (`aceG:`) then pops a notes modal. Submitting the modal
  POSTs `POST /api/v1/ace/requests/{id}/claim` as the service account, on behalf of the linked user, carrying the
  picked times + notes. The backend checks `ace.requests.claim`, checks the request is still open (data-dependent on
  top of the permission), records the claimer, and enqueues `ace_request_notify` (edit embed + notify EC) and, if the
  claimer is Discord-linked, `ace_claim_dm`.
- **Event-thread availability** (`evtavail:{green|yellow|red}:{event_id}`) → an NTMO/authorized DCC staffer presses
  one of three buttons on the event thread; the bot POSTs the linked user's availability to
  `POST /api/v1/integration/discord/availability/{id}` as the service account and replies ephemerally with
  confirmation, or an "unlinked"/"forbidden" message if the click can't be applied.
- **TMI "View structured"** (`tmiV:{tmi_id}`) → looks the TMI up again via the service account and replies
  ephemerally with its plain-English decoded description (see [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)).

## Open questions

- **Job delivery: poll vs. push — resolved, poll.** The bot polls the backend for due jobs (`OIS_POLL_SECS`, default
  5s). No push/gateway model was built.
- **DCC: threads vs. forums — resolved, threads.** `event_thread_create` creates a real Discord public thread
  (`ChannelType::PublicThread`), not a forum post.
- **Account linking entry point — moot.** Neither option below was needed: linking isn't bot- or site-driven at all,
  it's read directly from VATUSA's own member data (see [Config & account linking](#config--account-linking)).
- **Multi-guild routing for generic channel/role names — resolved by #194.** See **Multi-guild name
  collisions** in [Config & account linking](#config--account-linking).
