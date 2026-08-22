# IDST — Integrated Departure Scheduling

**IDST** is the console for issuing [CFR releases](/tmu/releases) to FCA-metered departures that are still on the ground. It pulls every ground departure that an [FCA](/tmu/fcas) is metering into one board, shows you each flight's proposed wheels-up time, and lets you freeze it with a click.

It complements the FCA map — you *draw and meter* a flow there, and *schedule its departures* here. IDST lives at **`/ops/idst`**.

## Your scope

The left panel is your **working scope**. Add the positions you're covering:

- **Tower · Airport** — type an ICAO/IATA (e.g. `KDCA`) and **Add**.
- **Facility · TRACON / ARTCC** — search a TRACON or ARTCC (e.g. `PCT`, `ZDC`, `N90`).

Scope resolves server-side to the **union of member airports** for whatever you added. Only **FCA-metered ground departures whose origin is in scope** show up in Flights to Work.

::: tip
Your scope is saved to your account and follows you between sessions and devices — set your tower(s) or TRACON once and it's there next time. An empty scope shows a prompt and no board.
:::

## Flights to Work

The center is two columns, with **UNSCHED / RELEASED / METERED** counts in the header:

- **Unscheduled** — metered ground departures with no frozen release yet, soonest metered crossing first. Each row shows the callsign, `DEP → ARR`, aircraft type, the metering **FCA** badge, and a **proposed EDCT** (with any delay, e.g. `+12m`). That proposed time is **advisory** — it drifts live with the flow until you commit it.
- **Released** — flights with a **frozen CFR**, shown in green as `RLSD 1236z` and sorted by wheels-up time.

## Working a flight

Select a flight to open the right panel. It shows the **CTA** (metered crossing time), the **proposed EDCT** (or the frozen wheels-up once released), the sequence number, and any delay.

Issuing a release needs **`flow.fca.update`** — read-only viewers see the board but not the buttons. There are two ways to release:

- **RDY — release earliest** — drops the flight into the **earliest open metered slot** and freezes the wheels-up time that results. Use this when the pilot is ready to go now.
- **Set `HHMMz`** — pins the flight so its **wheels-up is exactly the time you type**. Use this to hold a departure to a specific minute.
- **Cancel release** — on a released flight, returns it to Unscheduled.

::: tip Proposed vs. issued
The EDCT on an unreleased flight is advisory and moves with the flow. The moment you **RDY** or **Set**, it freezes — see [Release times](/tmu/releases) for what "frozen" means.
:::

## It stays in sync

A release you issue in IDST is the **same CFR** everyone else sees. It appears instantly on every other open board and in the airport [Departures](/tmu/releases) view's CFR column — and a release issued from the FCA map appears here — over the live update channel, with no refresh. (A short poll is the fallback if the connection drops.)

::: warning
Issue a release once. Whether you set it here or on the departures board, it's the authoritative wheels-up time a tower controller reads off the strip.
:::

## Permissions

- **View the board** — `flow.fca.read`.
- **Issue or cancel a release** — `flow.fca.update`.
