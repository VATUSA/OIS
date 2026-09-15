# Feature specs

One spec per built capability, describing what shipped — write a new one when a subsystem is
substantial enough to need its own reference (not every small feature needs one). Each follows the
template below so a spec always states its data model, its **exact permission entries**, its API
surface, and its Discord touchpoints.

## Index

| Spec                                             | Domain(s)            | Status                          |
|--------------------------------------------------|----------------------|---------------------------------|
| [access-control.md](access-control.md)           | access               | built                           |
| [api-keys.md](api-keys.md)                        | access (api_keys)    | built                           |
| [stats-replay.md](stats-replay.md)               | stats                | built                           |
| [events-workflow.md](events-workflow.md)         | events               | built (diverged from original spec) |
| [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)       | tmu                  | built (diverged from original spec) |
| [ace-support.md](ace-support.md)                 | ace, discord         | built (local roster since removed — sourced from VATUSA) |
| [discord-integration.md](discord-integration.md) | discord, integration | built |
| [flow.md](flow.md)                               | flow                 | built (diverged from original spec) |
| [aadc.md](aadc.md)                               | flow                 | built |
| [taxi-insights.md](taxi-insights.md)             | stats, flow          | built |
| [dashboard-boards.md](dashboard-boards.md)       | identity             | built |

Every spec above is built. The "diverged from original spec" ones have live migrations,
handlers, and routes, but the shipped shape differs from the original design — each doc
carries a note at the top explaining the realignment.

Event **posting, review, and approval remain in the current VATUSA website**; the
events spec above covers only OIS's operational window (prior / during / post).

## Template

```markdown
# <Feature>

## Problem
What we're replacing / why it exists. Link the originating request.

## Scope
In scope / out of scope for the first cut.

## Data model
Tables (schema.table), key columns, relationships, lifecycle/status enums.

## Permissions
Exact `segments.action` entries and which roles hold them (national vs ARTCC-scoped).

## API
Endpoints (method + path), request/response shape, who can call each.

## Discord
Outbound jobs enqueued, embed/thread shapes, interactions handled.

## Open questions
Decisions still needed from stakeholders.
```
