# Delays

Historical **taxi-out** (departure) and **arrival transit** delay data, filterable by airport,
runway, and procedure (SID for departures, STAR for arrivals) over a selectable time window
(default 24h).

![The Delays page filtered to KJFK departures over the last 24h, showing the median/average/p90 summary and a per-airport breakdown bar.](/screenshots/historical-delays.png)

Switch between **Departures** and **Arrivals** with the tab at the top. Pick an airport to narrow
the view, then a runway or procedure to drill further — each summary shows the median delay against
the airport's overall baseline, and a breakdown by SID or STAR so you can spot which procedure is
running slower than the rest.

::: info Permissions
Requires `stats.data.read` — the same access as the rest of [Historical](/historical/overview).
:::
