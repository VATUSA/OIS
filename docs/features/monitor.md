# Airspace Monitor — sector loading

> **Status: core built; not yet live.** The sector dataset, the bins, the alert ladder, Monitor Alert
> Parameters (MAPs) and consolidation are on `next` (#594, #596–#600). Nothing projects live flights into
> the bins yet and no endpoint serves sector loads (#701), and there is no Monitor page (#601) and no
> sector map layer (#602). Those sections say "pending" below rather than describing unmerged work.
> Epic: #593.

**Naming:** "Monitor" alone already means the **Taxi Monitor** (`backend/src/feed/taxi.rs`). This feature
is the **Airspace Monitor** (or Sector Monitor) everywhere, to keep the two apart.

## Problem

TMU staff can see airport demand (`aadc.md`) but not airspace demand: nothing says that a high sector
will hold more aircraft than one controller can work in forty minutes' time. The Monitor counts, per ATC
sector and per 15-minute bin, how many flights will be inside it at the busiest minute, compares that with
the sector's Monitor Alert Parameter, and colours the bin, the way vTBFM's sector monitor does.

## Scope

**Built:**

- Altitude-bounded ATC sector volumes, imported offline into Postgres.
- 3D containment, the 15-minute peak-occupancy binner, and the green / amber / red classifier.
- Per-sector MAPs (default 10), editable on the admin page **Monitor Alert Parameters**
  (`/admin/planning/sector-maps`, `web/src/pages/planning/sector-maps.tsx`).
- Sector consolidation (one controller working several sectors), API only.
- Polling vNAS for ERAM sector identities and which sectors are staffed.

**Not built yet:**

| Piece | Where it lands |
| --- | --- |
| Projecting live flights into the binner, and the two populations (airborne, proposed) | #701 |
| An endpoint serving sector loads and colours (`GET /api/v1/flow/monitor/{artcc}`) | #701 |
| The Monitor page | #601 |
| Sector volumes drawn on the map, and `flow.sectors.read` | #602 (PR #677) |
| Realtime fan-out of Monitor changes | #600 asked for it; nothing publishes yet |
| Joining vNAS staffing to sector rows, and expiring a stale vNAS snapshot | open (see below) |
| A UI for consolidation | none filed |

**Out of scope:** Class A–E airspace (the FAA `Class_Airspace` layer has real vertical limits but is not
ATC sectors), per #593.

## Data model

Three tables in `flow`, all ARTCC-keyed, and an in-memory cache of each.

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

**`flow.sector_map`** (`0116_sector_monitor_alert_parameters.sql`): per-sector MAPs, primary key
`(artcc, sector_id)`, `map > 0`. A sector with no row uses `DEFAULT_MAP = 10` (`feed/sectors.rs`), taken
from vTBFM, which tunes down from real high-sector values of about 16–20 for VATSIM traffic. There is no
foreign key to `flow.airspace_sector`, so a re-import never cascades away an override.

**There is no way to remove an override yet (#706).** The only write is an upsert. Entering the default on
a sector **with no row** is a no-op, but entering it on a sector that **has** an override stores the
default as an ordinary row. That sector then reports `overridden: true`, and stays at 10 even if
`DEFAULT_MAP` later changes. #706 makes entering the default delete the row. When it lands, this paragraph
becomes "setting the default removes the override".

**`flow.sector_consolidation`** (`0118_sector_consolidation.sql`): `(artcc, sector_id) →
target_sector_id`, `sector_id <> target_sector_id`. It is kept **flat**: a target is never itself a
source. Same ARTCC only, by construction.

**Caches** live on `AppState` and are refreshed by jobs in `backend/src/jobs.rs`:
- sectors every 5 minutes (`airspace_sectors_refresh`; the importer is a separate process, so a new
  import shows on the next tick);
- MAPs and consolidations every 30 seconds, and **force-reloaded on every write**.

A failed refresh keeps the current table.

## How it counts

**Containment** (`SectorVolume::contains`, `feed/sectors.rs`): laterally inside any ring, and
`base_alt_ft <= altitude < top_alt_ft`. The band is **half-open**, so a sector's top is the next stratum's
floor and a flight is never in two strata at once. An unknown altitude fails open: laterally inside counts.
It tests one altitude, the one supplied, not flow's "filed or current" rule.

**Bins** (`backend/src/feed/monitor.rs`, `sector_loads`):
- The count is the **peak concurrent occupancy**: the busiest minute's count of distinct flights inside the
  sector, not the number that pass through (vTBFM manual §10). A flight skimming a boundary, or passing
  between two pieces of a split sector, counts once.
- Bins are **absolute Zulu quarter-hours**: at 1407Z the first bin starts at 1400.
- Every sector is computed for a 6-hour horizon, whatever the UI shows.
- Each bin carries `active`, `proposed` and `combined`. `combined` is the busiest minute of the two
  together, **never `active + proposed`**.
- The two populations are defined (`Population`: airborne at ≥ 50 kt, and on the ground holding an issued
  departure slot) but nothing computes them yet (#701).

**Consolidation** files a worked sector's volumes under its target **before** counting. The combined row
therefore counts distinct flights across the **union** of the volumes, and can correctly read lower than
the sum of its parts. Its MAP is the target's own: one controller, one limit, never a sum.

**Alert ladder** (`backend/src/feed/monitor_alert.rs`, `sector_alert`), using a **strictly greater** test,
so a peak equal to the MAP never alerts:

| Colour | When |
| --- | --- |
| **Red** ("too late") | the airborne peak exceeds the MAP |
| **Amber** ("act now") | otherwise, the combined peak exceeds it |
| **Green** | neither |

Proposed load can only ever make a bin amber. The web maps the three to the `level-ok` / `level-watch` /
`level-over` tokens (`web/src/lib/monitor-alert.ts`). A guard test keeps literal colours out of every
Monitor web file (`web/src/lib/monitor-colors.guard.test.ts`). Colours are computed per request; there is
no alert-state table and no edge detection.

**Staffing** (`backend/src/feed/vnas.rs`, #595):
- ERAM sector identities come from `data-api.vnas.vatsim.net/api/artccs` (daily).
- Which sectors are staffed comes from `live.env.vnas.vatsim.net/data-feed/controllers.json` (every
  30 s): active, non-observer controllers with an ERAM sector id.
- Ids are zero-padded to two digits, and `ZHN` maps to `HCF`. Staffing is an attribute of a sector, never
  a filter.
- Nothing reads it yet.

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

## Permissions

| Permission | Lets you |
| --- | --- |
| `flow.monitor.read` | read sectors' MAPs and consolidations |
| `flow.monitor.update` | set a MAP; consolidate or release a sector |

**Scope:** both are ARTCC-scoped through the holder's facility grants. The write handlers check
`flow.monitor.update` against the path's ARTCC (`require_edit`, `backend/src/handlers/monitor.rs`). With no
update grant the call is a 401; with one for a different ARTCC it is a 403. The read responses' `editable`
flag uses the same check.

**Default roles:** both are granted to `VATUSA_STAFF`, `DCC_STAFF`, `EC`, `AEC` and `NTMO`
(`0116_sector_monitor_alert_parameters.sql`).

**Pending:** `flow.sectors.read`, which gates drawing sector volumes, arrives with #602.

## API

All routes are under `flow`, and `{artcc}` is uppercased.

| Method | Path | Permission | Body → result |
| --- | --- | --- | --- |
| GET | `/api/v1/flow/monitor/{artcc}/maps` | `flow.monitor.read` | `SectorMapsBody { editable, default_map, sectors: [{ sector_id, name, map, overridden }] }` |
| PUT | `/api/v1/flow/monitor/{artcc}/maps/{sector_id}` | `flow.monitor.update` | `{ map }` → 204. 400 if `map <= 0`; 404 if the sector isn't in this ARTCC; unchanged is a no-op 204 |
| GET | `/api/v1/flow/monitor/{artcc}/consolidations` | `flow.monitor.read` | `SectorConsolidationsBody { editable, consolidations: [{ sector_id, target_sector_id }] }` |
| PUT | `/api/v1/flow/monitor/{artcc}/consolidations/{sector_id}` | `flow.monitor.update` | `{ target_sector_id }` → 204. 400 for itself; 404 for a sector outside this ARTCC; 409 if it would loop. A target already worked elsewhere resolves to where it's worked, and sectors worked at the source follow it |
| DELETE | `/api/v1/flow/monitor/{artcc}/consolidations/{sector_id}` | `flow.monitor.update` | → 204, also when it wasn't consolidated |

Consolidation writes hold a per-ARTCC advisory lock, so concurrent saves can't build a chain.

**Pending:** `GET /api/v1/flow/monitor/{artcc}`, sector loads and colours (#701).

## Discord

None.

## Open questions

- **Will vNAS sector ids match stored ones?** vNAS ids are zero-padded, but the importer stores the
  dataset's `sector` string as-is. Whether the two line up needs checking when staffing is joined to rows.
- **How stale can staffing get?** A failed vNAS refresh keeps the last snapshot indefinitely, with no
  fetched-at time or expiry (raised in #595's review).
- **What type is `map`?** `sector_alert` takes `map` as `u32`, while `SectorLoad.map` is `i32`; the
  integrating handler (#701) has to convert.
