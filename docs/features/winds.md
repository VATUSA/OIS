# Winds aloft

How the trajectory model gets its wind and temperature, what the FB-only model got wrong, and how
much the change to NOAA RAP is worth on live traffic (#836). The model itself is
`backend/src/feed/trajectory.rs`; its callers are listed in `AGENTS.md` § The trajectory / ETA model.

## Data path

```
before (next at 2e8a5f8f):
  AWC windtemp text (6 regions × low/high) ─▶ jobs.rs refresh_winds_once (hourly)
     ─▶ winds::fetch: stations, 4 nearest within 600 nm ─▶ AppState.winds + stats.winds
     ─▶ each caller: route_headwind(path, ONE altitude) ─▶ one mean headwind
     ─▶ VerticalProfile::build: the same value on climb, cruise and descent

after:
  NOAA RAP awp130pgrb (AWS noaa-rap-pds, NOMADS mirror)
     ─▶ .idx ─▶ byte ranges for UGRD+VGRD and TMP at 14 levels (850–150 hPa)
     ─▶ rap::build_grid: decode (grib, JPEG2000), turn to true north, average onto a 1° grid
     ─▶ jobs.rs install_winds: RAP, else FB, else still air ─▶ AppState.winds + stats.winds
     ─▶ each caller: winds.along(path, route_len) ─▶ RouteWind (wind + temperature by distance and altitude)
     ─▶ VerticalProfile::build: every integration step and every query reads its own point
```

## Review of the FB-only model

These are the four limits the issue named, checked against the code on `next` before the change.

1. **Sparse, CONUS-only stations.** `winds::fetch` (`backend/src/feed/winds.rs:158`) read six
   regional FB tables (`REGIONS`, `:13`) and resolved each station through the airport database; the
   wind at a point was an inverse-distance-squared blend of the 4 nearest stations within 600 nm
   (`MAX_STATION_NM`, `BLEND_STATIONS`, `:17-19`). The live refresh in the measurement below loaded
   159 stations, about 120 nm apart on average over the CONUS, and nothing beyond 600 nm of one
   (Hawaii, most of Alaska, the oceanic approaches) had any wind.
2. **One headwind per flight.** `route_headwind` (`:130`) averaged the headwind over the route's
   segment midpoints at a single altitude, unweighted by segment length, and `trajectory.rs`
   applied that value to the climb, cruise and descent alike (`:314-336`, and `gs_at` at `:391`).
   Callers disagreed on the altitude: `predict::arrival_eta` (`predict.rs:245,250`),
   `build_candidates` (`handlers/flow.rs:1887,1999`) and `fix_predictions` (`:1623`) passed the
   filed cruise altitude, while `project_traffic` (`:1489`) and `sector_tracks` (`:270`) passed the
   aircraft's current altitude. A climbing departure was timed with its cruise headwind from the
   runway up, and one flight could carry two different headwinds on two pages.
3. **Temperatures dropped.** `decode_fb` (`:251`) read only the first four digits of a group, so a
   Mach cruise ran on the ISA speed of sound (`trajectory::mach_to_tas`, `:123`). At FL350 each
   10 °C off ISA moves an M0.84 TAS by about 11 kt.
4. **Valid time unknown.** The fetch URL (`:163`) sets no `fcst`, so AWC always answers with the
   6-hour product (`VALID` about 6 h after the data time, for use across a 7-hour window); the parser
   skips the header that says so, and the refresh (`jobs.rs:136`, hourly) recorded only when it ran.

## What changed

- **RAP is the primary field.** `feed/rap.rs` asks for the forecast hour of the newest published
  cycle that is valid at the hour nearest now, reads only the wind and temperature messages it needs
  by byte range (1.5 MB of a 17 MB file in the 2026-10-16 00Z analysis), and averages the 13 km points into 1° cells. Winds are
  grid-relative in RAP and are turned to true north by the bearing of the grid's y axis at each
  point. Coverage is RAP's 13 km North American grid (NCEP grid 130), from 16.3°N 126.1°W at its
  southwest corner to 55.5°N 57.4°W at its northeast corner.
- **Fallback.** `jobs::install_winds` tries RAP, then FB, then still air. A RAP failure that FB covers
  is still a failed run in the job registry. When neither answers, a field loaded within 3 h is kept
  and an older one is dropped for still air.
- **Per segment, at the profile's altitude.** `Winds::along` samples the field at most every 50 nm
  along the route the profile flies, each sample carrying the headwind on its leg's course at every
  level. The profile reads that at each step's own distance and altitude, so the climb, cruise and
  descent each see their own wind, and every caller uses the same rule.
- **Temperature.** A cruise, climb or descent Mach uses the field's temperature at that point; the IAS
  schedule and any point without a temperature stay on ISA.
- **Snapshots.** `stats.winds` rows written before the change (stations only) still load. A RAP
  snapshot is keyed on its valid time, so replicas loading the same cycle write one row.
- **What operators see.** `GET /api/v1/flow/data-status` reports `winds_source` and `winds_valid_at`,
  and the FCA data summary names the source (RAP winds, FB winds with a station count, or no winds).

## Measured error on live flights

**Method.** A throwaway harness (not in the repository) polled the VATSIM data feed twice, 60 s
apart, and kept flights established at cruise over the CONUS: at or above FL180, within 300 ft of the
filed altitude, level (under 200 ft change) and steady (under 10 kt change) between the two polls,
groundspeed 250 kt or more, a filed TAS from 300 to 600 kt, and inside both fields. The track came
from the two positions. For each flight it compared the observed groundspeed with the filed TAS minus
each source's headwind along that track at the flight's altitude. Each flight counts once.

The filed TAS is a plan value and pilots fly a Mach, and simulators run their own weather source, so
every row carries a TAS error the wind sources share. The absolute numbers are therefore an upper
bound on wind error; the comparison between rows is the result.

MEASUREMENT_TABLE

## Not done here

- #813 [surfaces feed the trajectory model different inputs] (To Do) covers the other inputs callers
  pass differently. The wind-altitude part is resolved by sampling along each profile.
- Points outside RAP's domain fly still air while RAP is the source; the FB stations there were
  outside its coverage as well.
