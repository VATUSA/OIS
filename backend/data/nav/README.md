# VATFLOW navigation data (FAA NASR / CIFP)

Pre-processed US enroute navigation data for route expansion in FCA Builder and Tower Departures.

## Rebuild

```bash
node scripts/build-nav-data.mjs --faa-cycle 2026-07-09
```

Uses FAA FIX/NAV/PFR CSV for the requested cycle and @squawk airways/procedures for enroute data (unchanged on 28-day change-notice cycles).

Fallback (prior @squawk-only snapshot):

```bash
node scripts/build-nav-data.mjs
```

Optional local NASR CSV override:

```bash
node scripts/build-nav-data.mjs --nasr-dir /path/to/CSV
```

Include `FIX_BASE.csv` or `FIX.csv`, `NAV_BASE.csv` or `NAV.csv`, `AWY_BASE.csv` or `AWY.csv`, and optionally `PFR_BASE.csv` or `PFR.csv` for preferred routes.

## Files

| File | Purpose |
|------|---------|
| `meta.json` | NASR cycle date, counts, CONUS bbox |
| `fixes.json` | Named fixes → `[[lat, lon], ...]` (multiple candidates when duplicated) |
| `navaids.json` | Navaid identifiers → coordinates |
| `airways.json` | Airway designation → ordered waypoint chain |
| `procedures.json` | SID/STAR identifiers → fix sequences |
| `preferred.json` | `DEP|ARR` → route string (from PFR when available) |

## Runtime

Loaded lazily by `shared/route-engine.js` on first route resolution.

Duplicate fix names are disambiguated by choosing the candidate nearest the previous route anchor.
