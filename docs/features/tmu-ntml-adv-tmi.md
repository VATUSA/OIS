# TMU — NTML / ADV / TMI

## Problem

Move NTML/advisory management onto the OIS site (today it's a legacy link only), let TMU staff generate and publish
TMIs, parse NTML/ADV into a plain-language section, and add an average-delay page. Discord integration for published
TMIs/ADVs.

## Scope

- **First cut**: NTML/ADV CRUD, TMI generation + publish, plain-language rendering, public + API exposure, Discord post
  on publish.
- **Later**: average-delay page (gate-out→wheels-up / entry→landing, color thresholds, filterable) — depends on the
  traffic data source (see [flow.md](flow.md)).

## Data model *(draft)*

- `tmu.ntml_entries` — id, artcc/facility, raw fields, parsed plain-language, timestamps.
- `tmu.advisories` — id, type, body, effective window, status.
- `tmu.tmis` — id, kind, params, plain-language render, `status` (draft→published), published_by, published_at.
- `tmu.delay_samples` — (later) feed for the average-delay page.

## Permissions

- `tmu.ntml.{read,create,update,delete}`
- `tmu.adv.{read,create,update}`
- `tmu.tmi.{read,create,publish,delete}`
- `tmu.delays.read`

Held by `TMU_NATIONAL` and ARTCC-scoped TMU staff.

## API

Read endpoints public (site + external API consumers); create/update/publish gated. TMIs and ADVs exposed via the
versioned API so other tools can consume them.

## Discord

On TMI/ADV publish, enqueue an outbound job to post an embed to the configured channel.

## Open questions

- NTML source format (s) to parse and the plain-language rules.
- Delay thresholds + color bands; data source for gate-out/wheels-up timings.
