# Restrictions & programs

Beyond FCAs and GDPs, OIS carries the everyday traffic-management restrictions a TMU issues: NTML
entries, ground stops, and named rate programs — all published to the public
[advisories board](/advisories/board) the moment they go live.

## NTML restrictions (TMIs)

A restriction is entered either **structured** (pick requesting/providing ARTCC, a spacing kind,
and a start/stop time) or as **raw NTML text** for anything the structured form doesn't cover.
Structured spacing has two kinds, spelled out rather than left as a bare abbreviation:

- **MINIT** — minutes-in-trail, a time gap.
- **MIT** — miles-in-trail, a distance gap.

A restriction is a **draft** until published; published restrictions appear live everywhere,
including the advisories board. Cancelling or letting one expire removes it from the active list
after a short grace period, but it stays in the historical record.

::: info Permissions
`tmu.tmi.create` to draft a restriction, `tmu.tmi.publish` to make it live, `tmu.tmi.delete` to
remove one.
:::

## Ground stops

A ground stop targets one **airport**, an optional **scope** (leave blank to stop everyone, or
narrow it to a facility/region), and an **until** time. Like restrictions, a ground stop is created
as a draft, then published to go live; it can be cancelled early or left to expire on its own.

::: info Permissions
`tmu.groundstop.create` to draft one, `tmu.groundstop.publish` to make it live,
`tmu.groundstop.delete` to remove one.
:::

## Rate programs

Separately from a formal [Ground Delay Program](/tmu/gdp), an airport can carry a standing
**program**: its Airport Acceptance Rate (AAR) plus a default **MINIT** (minutes-in-trail) or
**MIT** (miles-in-trail) spacing, with per-gate overrides for individual arrival fixes/STARs. This
is the lighter-weight, always-on metering baseline most airports run day to day; a GDP is the
heavier, time-windowed EDCT mechanism you reach for when demand genuinely exceeds it. A program's
AAR also feeds [the arrival demand chart](/tmu/aadc)'s reference line.

::: info Permissions
`tmu.program.read` to view, `tmu.program.update` to set the AAR, spacing, or per-gate overrides.
:::

## See also

- [Advisories board](/advisories/board) — where pilots and other facilities see everything active.
- [Glossary](/reference/glossary) — MIT, MINIT, and the rest of the TMU shorthand spelled out.
