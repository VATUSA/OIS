# Custom dashboards (boards)

> **Status: built.** A user-configurable grid of live widgets, distinct from the fixed home page.

## Problem

The home page is a fixed launchpad — useful as a jumping-off point, but nobody wants to check the
same 4 stat tiles every session if what they actually care about is one airport's arrival demand and
their facility's online ATC. Boards let a user build exactly that.

## Scope

**Built:** a board is a small JSON document (`DashboardState`: `widgets[]` + a `layout[]` of grid
cells) a user creates, edits (drag/resize/add/remove widgets), and optionally shares as a read-only
link. Boards can be grouped into named collections.

**Widget kinds** (the `Widget` union, `web/src/features/dashboard/types.ts`): `stat` (a single live
number), `view` (an embedded per-airport page, e.g. the arrival ladder), `map` (the flow map), `table`
and `chart` (any registered data source — arrivals, departures, restrictions, programs, etc. — as a
sortable table or a `@tanstack/charts` line/area/bar/scatter/pie plot with grouping, aggregation, and
static reference-line thresholds), `atc` (a facility's online controller roster), `facility_map` (an
embedded per-ARTCC map), `aadc` (see [aadc.md](aadc.md) — a bespoke kind, not a `chart`-kind data
source, since it needs stacked-by-dimension bars and a *data-driven* AAR reference line that the
generic chart grammar doesn't support), `text` and `divider` (layout-only, no data).

## Data model

`identity.dashboards` (migration `0035`) stores one row per board: `id`, owner, `name`,
`collection_id?`, the `data` JSON blob (`DashboardState`), `share_slug?`, `updated_at`.
`identity.dashboard_collections` groups boards by name per owner.

Adding a new widget kind is a web-only change (a new `Widget` variant, a render case, an
`AddWidgetMenu` entry) — the backend stores `data` as opaque JSON and never inspects widget shape.

## Permissions

Boards are personal: a user manages their own without a special permission (baseline authenticated
access). Viewing a **shared** board requires `auth.profile.read` (any signed-in account) but not
ownership or edit access.

## API

- `GET /api/v1/dashboards`, `POST /api/v1/dashboards`, `GET/PUT/DELETE /api/v1/dashboards/{id}`.
- `POST /api/v1/dashboards/{id}/share`, `DELETE /api/v1/dashboards/{id}/share` — mint/revoke a
  `share_slug`.
- `GET /api/v1/dashboards/shared/{slug}` — read-only fetch by slug (any signed-in user).
- `POST /api/v1/dashboards/shared/{slug}/copy` — clone a shared board into your own library.
- Collection CRUD under a parallel `/api/v1/dashboards/collections` surface.

## Discord

None.

## Open questions

None currently open.
