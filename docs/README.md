# OIS documentation

The design and plan for the OIS platform — the VATUSA operations system that replaces
the current backend, website, and flow tool.

## Start here

- **[../CONTRIBUTING.md](../CONTRIBUTING.md)** — how to file issues, the dev workflow, and PR
  expectations. Read this first if you're about to contribute.
- **[PROPOSAL.md](PROPOSAL.md)** — the leadership proposal: problem, solution, plan,
  and the ask. Read this first for the case for OIS.
- **[PLAN.md](PLAN.md)** — the phased build plan, locked decisions, and current status.

## Architecture

- [architecture/overview.md](architecture/overview.md) — the four services and how they fit together.
- [architecture/data-model.md](architecture/data-model.md) — Postgres schemas and key tables.
- [architecture/permissions.md](architecture/permissions.md) — the fine-grained permission model.
- [architecture/api-conventions.md](architecture/api-conventions.md) — REST shape, auth, errors, OpenAPI.
- [architecture/integrations.md](architecture/integrations.md) — VATSIM, VATUSA, Discord, email, service accounts.

## Operating OIS

- [deploy.md](deploy.md) — deploying to the test server / prod, the post-deploy health check,
  rolling back, and cutting a release.

## Feature specs

Each states the problem, data model, exact permissions, API surface, and Discord touchpoints.

- [access-control.md](features/access-control.md) — permissions + editor **(built)**
- [events-workflow.md](features/events-workflow.md) — event operations: coordination, staffing, sign-up, debrief (posting stays in the current VATUSA site) **(built)**
- [tmu-ntml-adv-tmi.md](features/tmu-ntml-adv-tmi.md) — traffic management: NTML / ADV / TMI, delays **(built)**
- [ace-support.md](features/ace-support.md) — ACE support requests **(built)**
- [flow.md](features/flow.md) — traffic management: flow programs, live traffic, delay feed **(built)**
- [discord-integration.md](features/discord-integration.md) — the bot and the outbound-job queue **(built)**
- [features/README.md](features/README.md) — index + the spec template

## Status at a glance

Built and running: VATSIM auth + sessions + `/me`; the permission model + access editor
(with per-domain ARTCC scope enforcement started); TMU (TMIs, ground stops, rate programs,
GDPs); flow (FCAs, routes, runway configs, facility map, IDST, live map traffic); the
events cache + per-event planning; ACE support requests (roster, claim, Discord embeds +
reminders); the Discord bot (outbound-job queue, event threads, TMI posts, ACE claim
flow); stats collection + replay; the live VATSIM feed; the audit log; service-account
admin; user dashboards; and the additive realtime WS layer. Everything is sequenced in
[PLAN.md](PLAN.md).
