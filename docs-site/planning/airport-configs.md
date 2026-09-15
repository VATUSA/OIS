# Airport rates & configs

Beyond a single AAR number, an airport can carry a library of **named runway configurations** — each
with its own AAR/ADR and a **favored-wind rule**, so the system (and the [arrival demand
chart](/tmu/aadc)) can pick the one that matches current conditions automatically.

## Building a config

For each named configuration, set:

- **AAR / ADR** — the arrival and departure rates this configuration supports.
- **Landing runways** — which runways this configuration uses.
- **Favored wind range** — the surface wind direction band this configuration is built for (e.g.
  `340°`–`020°` for a north flow). Ranges can wrap through 360°.
- **Calm default** — mark at most one configuration per airport as the fallback for light or
  variable wind.

## How the favored config is picked

Given the current forecast wind, OIS picks the first non-calm configuration whose wind range
contains it; if none matches (or the wind is calm), it falls back to the calm-default
configuration, or the first one on file if there isn't one. This is the same rate the
[arrival demand chart](/tmu/aadc)'s reference line uses.

::: info Permissions
Requires `events.plan.read` to view; editing is `events.config.update`, scoped to the airport's
owning ARTCC.
:::
