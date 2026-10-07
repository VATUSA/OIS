# Sector dataset — altitude-bounded ATC sector volumes

> **Status: dataset kept; the Airspace Monitor was removed (#719).** The Monitor (#593, #594–#602) is
> gone: its MAPs, consolidation, alert ladder, page and `flow.monitor.*` permissions were dropped by
> migration 0125. What stays is the sector **dataset**, its importer, and the admin sector viewer, as the
> input to a rebuilt feature (its own epic). Known coverage gaps: #728.

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
above. It has no endpoint yet; #725 serves it.

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
- **Flat on every write** (`repos::sector_consolidations::consolidate`, one transaction, serialised per
  ARTCC). A target that is itself worked elsewhere resolves to where it is worked. Sectors worked at the
  source move with it: 18 at 41, then 41 at 20, leaves 18 at 20.
- **Read**: `GET /api/v1/flow/sector-consolidations/{artcc}` (`flow.sectors.read`), with `editable`.
- **Write**: `PUT /api/v1/flow/sector-consolidations/{artcc}/{sector_id}` with `target_sector_id`, and
  `DELETE` on the same path to release. Both need `flow.sector_consolidations.update` for that ARTCC,
  which is separate from `flow.sector_limits.update` and granted to the same five groups. A release
  isn't checked against the dataset, so one left behind by a re-import can still be cleared.
- **Cache**: `AppState::sector_consolidations`, refreshed every 30 s by `sector_consolidations_refresh`.
  Every write force-reloads it, so even a no-op answers with the stored arrangement rather than a
  cache another replica's write has left behind. A write that changes anything also publishes
  `flow.sector_consolidations`.

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
