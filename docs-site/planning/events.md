# Planning an event

OIS doesn't replace the VATUSA website for **creating, reviewing, or posting** an event — it owns
the **operational layer around** one: everything a TMU and host facility coordinate before, during,
and just after it. Events themselves sync in automatically from VATUSA; open one from the events
list to start planning.

An event's planning page is a set of tabs:

## Airports & rates

Set a planned Airport Acceptance Rate (AAR) and Departure Rate (ADR) per airport for the event —
the numbers a TMU expects to run on the day, ahead of actually issuing a live
[program](/tmu/restrictions#rate-programs) or [GDP](/tmu/gdp).

## Facility support

Each participating facility declares its own support level (e.g. required) and notes, so hosts and
other facilities can see staffing commitments at a glance. A facility manages its own row.

## TMI packages

Bundle planned traffic-management items — programs, restrictions, ground stops — into a named
**package**. A package is a draft until you **activate** it, which turns every item in it into the
real, live thing in one step (and **deactivate** reverses it). This is how a whole event's traffic
plan goes live on cue instead of being set up item by item as the event starts.

## FCAs

Build and preview Flow Constrained Areas for the event ahead of time, the same builder as the
live [FCA map](/tmu/fcas).

## ACE

Request ACE (traffic-management support) staffing for the event; see the dedicated ACE support
workflow for how a request is claimed and tracked.

## Availability

Staff who respond to the event's Discord coordination-thread availability buttons show up here —
a live read-out of who's said they can work it, without leaving Discord to check.

## Stats & debrief

After the event, review captured statistics for the window and write a debrief entry.

::: info Permissions
Viewing any tab requires `events.plan.read`. Editing is split by tab: `events.rate.update`
(airport rates, facility-scoped), `events.support.update` (facility support, facility-scoped),
`events.plan.update` (TMI packages, FCAs), `events.discord.publish` (posting the coordination
thread), `events.debrief.create` (writing a debrief).
:::
