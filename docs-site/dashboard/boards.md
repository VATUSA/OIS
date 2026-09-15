# Custom boards

Beyond the fixed [home page](/dashboard/home), you can build your own dashboards — a grid of live
widgets you arrange yourself. Open **My boards** from the user menu.

## The board library

Your boards are listed here, optionally grouped into named **collections** (create, rename, or
delete a collection from this page). Open a board to view or edit it; a board you don't own but
that's been shared with you appears as a read-only link instead.

## Building a board

![A custom board with two stat-tile widgets — pilots online and active metering programs.](/screenshots/dashboard-boards.png)

Inside a board, **Edit** toggles editing: drag widgets to rearrange, resize from a corner handle,
or remove one. **Add widget** opens a menu of everything you can place:

- **Stat tiles** — a single live number (e.g. pilots online, active metering programs).
- **Airport views** — an embedded page for one airport (e.g. the arrival ladder, taxi field), bound
  to an ICAO you pick when adding it.
- **Tables** — any of the app's live data sources (arrivals, departures, restrictions, programs, …)
  as a sortable/filterable table, optionally scoped to one airport or a whole ARTCC/TRACON.
- **Charts** — the same data sources plotted as a line/area/bar/scatter/pie chart, with grouping,
  aggregation, and optional reference-line thresholds you configure per widget.
- **Arrival demand chart (AADC)** — a bucketed forward-demand chart for one airport; see
  [Arrival demand chart](/tmu/aadc). Configured per-widget (airport, bucket size, breakdown).
- **Online ATC positions** — a facility's live controller roster.
- **Maps** — an embedded flow map, or a facility map for one ARTCC/TRACON.
- **Text / heading and dividers** — for labeling and organizing a busy board.

Every widget updates live on its own schedule — there's nothing to manually refresh.

## Sharing a board

A board can be **shared** as a read-only link (from the board's menu) — anyone signed in to OIS with
the link sees the current state of your board without needing edit access or ownership (a shared
link still requires a VATSIM sign-in, just not any special permission). Turning sharing off
invalidates the link.
