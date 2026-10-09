# Sector dataset — altitude-bounded ATC sector volumes

> **Status: rebuilt (#720).** The old Airspace Monitor (#593, #594–#602) was removed in #719 (migration
> 0125). The sector **dataset**, its importer and the admin sector viewer stayed, and the rebuild reads
> them: occupancy (#721), limits (#722), consolidation (#723), TRACON strata (#726)
> and the Operations page (#725), redrawn as vTBFM's monitor (#794). #724's `SectorGrid` was removed in #794. Known coverage gaps: #727, #728.

## Data model

**`flow.airspace_sector`** (`backend/migrations/0111_airspace_sector.sql`), one row per sector **volume**.
A sector can be several volumes.

| Column | |
| --- | --- |
| `artcc`, `sector_id`, `volume_id` | `unique (artcc, volume_id)`; `name` is nullable (the importer leaves it null) |
| `tier` | `low`, `high`, `ultra_high` or `approach` |
| `base_alt_ft`, `top_alt_ft` | feet; `check (base_alt_ft < top_alt_ft)`, base ≥ 0 |
| `rings` | jsonb: closed rings of `[lat, lon]`, one per polygon part, no holes |
| `source`, `source_cycle`, `imported_at` | provenance: dataset name, exact revision, load time |

Volumes are validated on the way in (`backend/src/feed/sectors.rs` `validate_volume` / `validate_ring`):
a positive band, at least one ring, each closed with ≥ 4 points, in range, no revisited vertex, no
crossing edges. The only writer is `repos::airspace_sectors::replace_artcc`, which replaces one ARTCC's
volumes in a transaction.

**Cache:** `AppState::airspace_sectors`, reloaded every 5 minutes by `airspace_sectors_refresh`
(`backend/src/jobs.rs`). The importer is a separate process, so a new import shows on the next tick. A
failed refresh keeps the current table.

## Containment

**Containment** (`SectorVolume::contains`, `feed/sectors.rs`): laterally inside any ring, and
`base_alt_ft <= altitude < top_alt_ft`. The band is **half-open**, so at a shared boundary altitude a
sector's top is the next stratum's floor and stacked strata never both claim it. Bands can still
*overlap*: the source's enroute Lows start at the surface over the TRACONs, so KATL at 3,000 ft is inside
both `ZTL 70` (0–4,000) and `ZTL 59` (0–23,000). An unknown altitude fails open: laterally inside counts.
It tests one altitude, the one supplied, not flow's "filed or current" rule.

**Counting: TRACON precedence (#726)** (`SectorTable::counting`, `feed/sectors.rs`). Containment is not
counting. Of the volumes that contain a fix, if any has tier `approach` (`APPROACH_TIER`), only the
approach volumes count it; otherwise every containing volume does.

- Precedence is **global, not per ARTCC**: a ZJX approach volume suppresses a ZMA enroute volume over it.
  Callers pass the whole table, never one facility's slice.
- It holds only while the fix is in the approach band: it releases at the TRACON's top (the half-open
  band puts the top in the stratum above) and laterally outside the TRACON.
- A fix with an unknown altitude laterally inside a TRACON counts in the TRACON only.
- Approach-vs-approach and enroute-vs-enroute overlaps between different sectors are not resolved; each
  counts.

**Source gap: ZSE has no TRACON volumes.** At the pinned vTSD commit ZSE has Low and Ultra High volumes
only, and KSEA and KPDX at 3,000 ft are inside no volume at all. The TRACONs exist in the same repo's
`Data/tracon-boundaries.json`, but without altitudes, so OIS invents nothing; the data is #727's. The
engine keeps the two states apart: a quiet TRACON is a row of zeros, while ZSE has no approach row at
all, which the TRACON view (#725) should report as "no TRACON sector data" rather than render as quiet.

## Sector occupancy (#721)

The engine behind the sector-forecasting epic (#720). It is pure and DB-free, reading the cached table
above. `GET /api/v1/flow/sector-demand/{artcc}` serves it (Operations page, below).

- **Cell value** (`feed/sector_load.rs`, `sector_loads`): for each sector and 15-minute bin, the **peak
  one-minute concurrent occupancy**. Each minute it counts the distinct flights counted by any of the
  sector's volumes (Counting, above), and the cell is the busiest of the bin's fifteen minutes. It is not throughput: 40 flights
  transiting a sector, never more than 3 at once, read 3. A boundary skim within a minute, or a crossing
  between a sector's pieces, counts once.
- **Bins** are absolute Zulu quarter-hours, 24 of them (6 h). The first is the quarter-hour containing now:
  at 1407Z it starts at 1400.
- **Two populations**, returned apart per bin. **Active** means airborne (≥ 50 kt), projected from where
  the flight is. **Proposed** means on the ground holding a locked wheels-up, projected from it. `combined`
  is the busiest minute of both together, never active-peak plus proposed-peak.
- **Projection** (`feed/sector_tracks.rs`, `project_tracks`) is a read-only caller of the trajectory model.
  It makes the same calls as the map's predicted-traffic projection, so it adds no second predictor.
- **Wheels-up precedence** (`repos::flow::locked_wheels_up`): a flight can hold an issued CFR, a release in
  each FCA it crosses, and a GDP slot. The engine takes the **latest** of the following:
  - its CFR (`tmu.issued_cfrs.wheels_up`);
  - its releases in live FCAs (enabled, not deleted);
  - its slot EDCT in a `published` GDP.

  The latest is the binding constraint, since a flight held for a later release can't satisfy an earlier
  one. The flight advisory and the TMU departures list (#732) use the same rule.

## Sector limits (#722)

A sector's occupancy is judged against its **limit**: how many aircraft one controller can work there.
The limit is shared, so everyone watching an ARTCC sees the same colours.

- **Default 10** (`feed/sector_limits.rs`, `DEFAULT_LIMIT`). Only overrides are stored, in
  `flow.sector_limit` keyed per `(artcc, sector_id)`. Setting a sector back to 10 deletes its row.
- **Level** (`level`), strictly greater than: **over** when the active peak alone exceeds the limit,
  **watch** when only `combined` does, else **ok**. A peak equal to the limit is ok.
- **Read**: `GET /api/v1/flow/sector-limits/{artcc}` (`flow.sectors.read`) lists the ARTCC's sectors
  from the cached dataset, with `editable` for the caller. An ARTCC with no sector data returns no
  sectors, not a 404.
- **Write**: `PUT /api/v1/flow/sector-limits/{artcc}/{sector_id}` needs `flow.sector_limits.update`
  for that ARTCC. A TMU at one facility gets a 403 on a neighbour's sectors. Zero or a negative limit is
  a 400, an unknown sector is a 404, and an unchanged value writes nothing. A refusal never touches an
  existing override. The comparison is made against the stored row, not the cache.
- **Cache**: `AppState::sector_limits`, refreshed every 30 s by `sector_limits_refresh`. A write
  force-reloads it and publishes `flow.sector_limits`, so viewers recolour at once.

## Sector consolidation (#723)

Sectors worked at one position combine into one row. Stored in `flow.sector_consolidation`
(migration 0128) as `(artcc, sector_id) → target_sector_id`.

- **Union, never a sum** (`sector_loads`, `feed/sector_load.rs`): a consolidated sector has no row of its
  own. Its volumes are filed under the target's row **before** counting, so the combined row counts
  distinct flights per minute across all the airspace. An aircraft crossing from a source into the target
  within a minute counts once. A combined row can read lower than the sum of its parts, and that is
  correct. `SectorLoad::consolidated` lists the sources, so the row can be labelled.
- **The target's limit** (`row_limit`, `feed/sector_consolidations.rs`): one controller, one workload.
  Never the sum of the sources' limits and never their maximum.
- **Rules**: same ARTCC only, since both sectors are looked up in the path's ARTCC (another ARTCC's
  sector is a 404). A sector worked at itself is a 400. A loop (a at b, then b at a) is a 409. Neither
  refusal writes anything.
- **Flat on every write** (`repos::sector_consolidations::consolidate` and `apply_batch`, one transaction, serialised per
  ARTCC). A target that is itself worked elsewhere resolves to where it is worked. Sectors worked at the
  source move with it: 18 at 41, then 41 at 20, leaves 18 at 20.
- **Read**: `GET /api/v1/flow/sector-consolidations/{artcc}` (`flow.sectors.read`), with `editable`.
- **Write**: `PUT /api/v1/flow/sector-consolidations/{artcc}/{sector_id}` with `target_sector_id`, and
  `DELETE` on the same path to release. Both need `flow.sector_consolidations.update` for that ARTCC,
  which is separate from `flow.sector_limits.update` and granted to the same five groups. A release
  isn't checked against the dataset, so one left behind by a re-import can still be cleared.
- **Batch write** (#794): `PUT /api/v1/flow/sector-consolidations/{artcc}` with `{ "into": { "<sector>":
  "<target>" | null } }`, same permission and scope. All or nothing in one transaction
  (`repos::sector_consolidations::apply_batch`): releases first, then saves in sector order, each flattened
  as above, and the first refusal (400, 404 or 409, a loop between the batch's own entries included)
  writes none of it. A null entry releases without a dataset check, like `DELETE`; two keys that trim to
  one sector, or more than 200 entries, are a 400. One `flow.sector_consolidations` publish for the whole batch. The Sector
  Monitor's menu sends every consolidation command through it.
- **Cache**: `AppState::sector_consolidations`, refreshed every 30 s by `sector_consolidations_refresh`.
  Every write force-reloads it, so even a no-op answers with the stored arrangement rather than a
  cache another replica's write has left behind. A write that changes anything also publishes
  `flow.sector_consolidations`.

## Operations page (#725): the serving contract

The page under **Operations** draws one ARTCC's demand as an enroute table and a TRACON table, plus a
collapsed, view-only table per neighbour. Everything it draws comes from one read.

**`GET /api/v1/flow/sector-demand/{artcc}`** (`handlers/sector_demand.rs`), gated `flow.sectors.read` like
the limit and consolidation reads. The ARTCC is case-insensitive. There is no range parameter: the server
always sends all 24 bins, and the 2–6 h slider and the alert filter slice them client-side, with no refetch.

| Field | |
| --- | --- |
| `status` | `no_sector_data`, `pending` or `ready` (below) |
| `cycle_at` | the VATSIM publish the counts were projected from (its `update_timestamp`); null unless `ready` |
| `bin_minutes`, `bin_starts_ms` | 15, and each bin's start as epoch ms on absolute Zulu quarter-hours, the first containing `cycle_at` |
| `default_limit` | what an unset limit reads |
| `limits_editable`, `consolidations_editable` | the caller's `flow.sector_limits.update` / `flow.sector_consolidations.update` scope covers this ARTCC |
| `neighbours` | the bordering OIS ARTCCs, sorted (`feed::neighbors::tier1` over the active `org.facilities`) |
| `enroute`, `tracon` | `{ has_sector_data, rows }`: Low/High/Ultra High rows, and Approach Control rows |

Each row is `sector_id`, `name`, `tier`, `limit`, `limit_overridden`, `consolidated` (the sources worked
at it) and one bin per `bin_starts_ms`: `active`, `proposed`, `combined` and `level`.

- **Levels are the server's.** Each bin's `level` is `feed::sector_limits::level` against `row_limit`, so a
  combined row is judged by its target's limit and a peak equal to the limit is `ok`. The page colours
  from `level` and never recomputes it.
- **Consolidation applied.** The engine is called with `AppState::sector_consolidations`, so a source has
  no row and its target's row lists it in `consolidated`.
- **The three states, none of them an empty grid.** `no_sector_data`: the dataset has no volume for the
  ARTCC at all (ZLA, ZAN, HCF until #727); the page names it, "No sector data for ZLA". It wins over
  a missing feed snapshot, and an empty dataset reads this way for every ARTCC once it has been loaded.
  `pending`: the sector table hasn't been loaded since startup (`AppState::airspace_sectors_loaded`), so
  its emptiness says nothing yet, or there is no feed snapshot yet; nothing has been counted. `ready`: counted. Within `ready`, a table with `has_sector_data: false` is a
  gap in the data (ZSE has no TRACON volumes), distinct from a quiet table of zero rows.
- **Neighbours are view-only on this page.** The flags describe the caller's scope at the requested
  ARTCC, so a national TMU reads `true` for a neighbour too. The page ignores them for neighbour tables;
  the writes stay scoped server-side (a facility TMU gets 403 at a neighbour).
- **Computed once per publish, not per request.** Projection (`sector_tracks::project_tracks`, boxed to
  the ARTCC's volumes) and binning (`sector_loads`, over the **whole** table for TRACON precedence) run
  under `spawn_blocking` once per ARTCC per change, into `AppState::sector_demand`
  (`handlers::sector_demand_cache`), and every viewer, table and neighbour read of that ARTCC is served
  from it. An entry is reused while the VATSIM publish (the snapshot's `source_timestamp`, not the
  snapshot: the poller installs a new one on every 2 s poll, repeats included), the airport, nav, profile,
  wind and sector tables (by identity), the consolidations, the excluded callsigns and the grounded flights' wheels-up (by value)
  are the ones it was built from. A consolidation write force-reloads its cache, so the next read
  re-projects; a limit write only re-judges the cached rows; a refresh job's reload that changes nothing
  costs nothing. The first read after a change computes and concurrent reads of the same ARTCC wait for it
  (single-flight); nothing is computed for an ARTCC nobody reads, so the feed tick does no extra work.
- **Per request,** the database is read for the caller's two edit scopes, the locked wheels-up of the
  grounded flights and prefiles (`repos::flow::locked_wheels_up`, part of the key) and the facility list
  for the neighbours. Flights any facility has excluded count nowhere. The projection and its bins start
  from the publish time, not the fetch's or the request's, so a repeated poll can't move them and a
  limit-only re-render keeps its projection's clock.
- **The walk skips what the box can't see.** A track is resolved minute by minute with the trajectory
  model's `distance_after`, but only for the minutes on legs that can reach the ARTCC's box (each leg cut
  into 20 nm pieces and bounded around its great circle), found by bisecting the minutes. The fixes are
  bit-for-bit those of the every-minute walk (`sector_tracks`' tests pin it).

**Realtime.** No new topic. The web query key is `["sector-demand", artcc]` (`useSectorDemand`,
`web/src/features/sector-demand/sector-demand.ts`), and `web/src/lib/realtime.ts` refetches it on:

- `feed.tick` (a new cycle; at most once a minute);
- `flow.sector_limits` (cells recolour) and `flow.sector_consolidations` (rows merge or split), at once:
  each is a deliberate, rare edit someone is waiting to see;
- `flow.release`, `flow.cfr`, `tmu.gdp` and `flow.fca` (a wheels-up moved, so the proposed counts did),
  **coalesced into the next `feed.tick`** (`COALESCED_KEYS`). These can arrive several times a minute
  while a program runs, and each wheels-up change is a fresh projection per open ARTCC on the server.
  Held until the tick, a burst becomes one refetch that every client makes against the same new
  snapshot, so the server normally projects once per ARTCC per publish however many nudges came (a
  wheels-up committed between two clients' refetches on one tick costs a second); the change
  shows within one publish (~15 s). With no tick, `COALESCE_MS` (20 s) refetches it anyway. A
  reconnect's catch-up drops anything held.

## Operations page (#725): the page

**Operations → Sector Monitor** (`/ops/sectors`, `web/src/pages/sector-monitor.tsx`, components in
`web/src/features/sector-demand/`). The nav item and the page are gated on `flow.sectors.read`, the
endpoint's own gate; without it the page says so and asks for nothing. A rostered controller holds it
through `CONTROLLER` (migration 0129); a grant at their facility reads every ARTCC, edits none.

- **The set follows the facility selector and nothing else.** It opens on the viewer's VATUSA home
  facility and the pick is not remembered, so a controller who moves facilities does not keep the old
  one's tables. The set is keyed by facility, so a switch replaces every table, control and neighbour.
- **It looks like vTBFM's Sector Monitor** (#794), a named exception in `DESIGN.md`: a beige body, one
  bevelled table per facility and stratum (`ZLA`, then `ZLA TRACON`), then each neighbour's two. Rows are
  the sector (`ZLA25`, with a trailing `+` when others are worked at it) and its MAP (`10/10`), then one
  cell per bin holding the combined peak, green/yellow/red from the server's `level`. Time labels are a
  bottom footer (blank, `MAP`, then `HHMM`). A cell's tooltip reads `ZLA25 0415Z · peak 3 (airborne 2)
  vs MAP 10 · red`. The OIS shell and the facility picker stay as they are. Every colour literal is in
  `web/src/features/sector-demand/vtbfm-palette.ts`, the only file `colours.guard.test.ts` exempts.
- **Toggle:** ▼/▶ on the facility's own table folds away only its controls; on a neighbour's it hides
  controls and grid together. Neighbour tables start collapsed and are fetched only once one is opened.
- **Controls, per table:** `Time Range:` 2–6 h in whole hours (default 4 h), and `Show if alerted in
  next:` 1.00–6.00 h (**on by default at 2.00 h**, unlike vTBFM). The span is judged over all six
  computed hours, independent of the range, and both only slice what the server sent, never refetching.
- **Remembered per browser,** per ARTCC and table, in `localStorage` under
  `ois.sectorMonitor.<ARTCC>.<enroute|tracon>.<open|range|alertOnly|alertSpan|collapsed|order>`. Every
  access is guarded; with storage blocked, the defaults apply.
- **MAP edit** (own facility, `limits_editable`): click the MAP cell for an inline input; Enter or blur
  commits only a changed positive number, Escape cancels. Optimistic, rolled back with "Could not save
  MAP for ZLA25 — check TMU access / connection." on a refusal. Limits are edited only here; the web
  never calls `GET /flow/sector-limits/{artcc}`.
- **Right-click menu, the consolidation editor (#792)** (own facility, `consolidations_editable`; absent
  otherwise and on every neighbour): Move Row Up/Down (row order is per browser); Consolidate ▸ All into
  T, All into T Except Consolidated, into T ▸ (a checklist that stays open); Deconsolidate ▸ All from T,
  All in ZLA, from T ▸. Every write goes through the batch `PUT` above as one request, optimistic, rolled
  back on a refusal with a line naming the sector: "ZLA25 can't be consolidated into itself." (400), "Can't
  consolidate ZLA25 into ZLA30: ZLA30 is worked at ZLA25." (409), "ZLA99 is not one of ZLA's sectors."
  (404), "You can't change ZLA's consolidations." (403), else "Could not save the consolidation — check
  TMU access / connection." The row merges or splits without a reload: the success refetches, and
  `flow.sector_consolidations` reaches every other viewer.
- **States:** "Waiting for the first sector-monitor cycle…" before the first read and while `pending`;
  "No sector data for ZLA" for `no_sector_data`, in one table; "No TRACON sector data for ZSE" for a
  table without volumes; "No sectors for ZLA." / "No TRACON sectors for ZLA." for a table with no rows;
  "No ZLA sectors alerting in the next 2.00 h." ("No ZLA TRACON sectors…") when the filter hides every
  row. None of them is a grid.

## The sector dataset

### Where it comes from

`airspace-sector-importer` (`backend/src/bin/airspace_sector_importer.rs`) is run offline:

```
RUST_LOG=info cargo run -p ois-backend --bin airspace-sector-importer [-- --artcc ZDC]
```

- **What it does:** it fetches `Data/sectors.json` from the
  [`Virtual-Traffic-Situation-Display/vtsd`](https://github.com/Virtual-Traffic-Situation-Display/vtsd)
  repository at a **pinned commit**, `f33ef73f21091e71456e45cfe9019ddf3ba76247`, and loads it into
  `flow.airspace_sector`.
- **What each row records:** `source = 'vtsd sectors.json'` and `source_cycle = <that commit>`. The
  cycle is a git revision, not an AIRAC date. To move to a newer revision, change `SOURCE_REF` and
  re-run.
- **Mapping:** tiers map `Low`, `High`, `Ultra High` and `Approach Control` to the four tiers above.
  Positions flip from `[lon, lat]` to `[lat, lon]`.
- **Invalid volumes:** a volume that fails validation is skipped and logged, never repaired.
- **Last import:** 1,568 volumes across 19 ARTCCs, with ZTL `07007` skipped because its floor is above
  its ceiling (#594).

### Licence position

The source repository is **CC BY-NC-SA 4.0**; OIS is **MIT**.
- Its authors gave permission to use it. The operator confirmed that the permission covers **loading the
  data into OIS's database**, which is what the importer does (recorded on #594). It is the data loaded,
  not merely a reference consulted.
- Neither the file nor any of its code is committed to this repository. The importer fetches it at run
  time.
- If OIS ever **serves** this geometry (the sector map layer, #602, or a containment endpoint), re-check
  the share-alike and attribution terms at that point.

### Where altitude-bounded sector geometry does not come from

Searched before choosing the dataset above (#594), so that nobody re-runs the hunt. **No public FAA or
vNAS source gives altitude-bounded ATC sector volumes.**

| Source | What it has |
| --- | --- |
| FAA ArcGIS `Airspace` layer (the org `faa_surface_importer.rs` uses) | `TYPE=ARTCC` is centres only: one row per ARTCC, `SECTOR_TXT` null, vertical fields the `-9998` no-data sentinel |
| NASR 28-day subscription | No sector subject: `SEC.zip`, `SECTOR.zip`, `ARTCC.zip` all 404; only `ARB` (boundary segments) and `ATS` exist |
| NASR AIXM 5.1 (55 MB) | Only `APT_AIXM`, `AWY_AIXM`, `NAV_AIXM`, `AWOS_AIXM`; no airspace file at all |
| SDAT | `sdat.faa.gov` does not resolve publicly |
| vNAS (`data-api.vnas.vatsim.net/api/artccs/{id}`) | Sector **identities** only (e.g. 54 for ZDC, as `{id, sectorId, name, isFromEramData}`). `airspaceConfigurations` is empty; the video maps are incomplete linework with no altitudes |

## Viewer and permission

The admin sector viewer (**Admin → Flow → Sectors**, `web/src/pages/flow/sectors.tsx`) draws the volumes
from `GET /api/v1/flow/airspace/sectors` (`backend/src/handlers/airspace_sectors.rs`). It is gated by
`flow.sectors.read` (migration 0120), and is how anyone checks an import.
