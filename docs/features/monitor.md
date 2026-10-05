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
`base_alt_ft <= altitude < top_alt_ft`. The band is **half-open**, so a sector's top is the next stratum's
floor and a flight is never in two strata at once. An unknown altitude fails open: laterally inside counts.
It tests one altitude, the one supplied, not flow's "filed or current" rule.

## Sector occupancy (#721)

The engine behind the sector-forecasting epic (#720). It is pure and DB-free, reading the cached table
above. It has no endpoint yet; #725 serves it.

- **Cell value** (`feed/sector_load.rs`, `sector_loads`): for each sector and 15-minute bin, the **peak
  one-minute concurrent occupancy**. Each minute it counts the distinct flights inside any of the sector's
  volumes, and the cell is the busiest of the bin's fifteen minutes. It is not throughput: 40 flights
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
