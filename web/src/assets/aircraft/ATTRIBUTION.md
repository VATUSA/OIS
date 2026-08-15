# Aircraft silhouette icons

These top-down aircraft silhouettes (293 SVGs, one per ICAO type designator) are
vendored from the **VATSIM Radar** project:

- Source: https://github.com/VATSIM-Radar/vatsim-radar
- Path in source: `app/assets/icons/aircraft/`
- License: **Creative Commons Attribution-NonCommercial 4.0 International (CC BY-NC 4.0)**
  — https://creativecommons.org/licenses/by-nc/4.0/

The type → icon mapping in `web/src/lib/aircraft-icons.ts` is likewise adapted
from VATSIM Radar's `getAircraftIcon()` (`app/utils/icons.ts`).

## Terms

CC BY-NC 4.0 permits use, sharing, and adaptation for **non-commercial**
purposes with **attribution**. OIS is a non-commercial VATUSA / VATSIM community
project. Attribution is surfaced in the app: the flow map's attribution line
credits "planes: VATSIM Radar (CC BY-NC)".

If OIS's licensing status changes, or these assets are reused in a commercial
context, this dependency must be re-evaluated.
