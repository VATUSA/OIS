# Feature specs

One spec per capability. Phase 1 fills these in before the matching backend domain is built. Each follows the template
below so a spec always states its data model, its **exact permission entries**, its API surface, and its Discord
touchpoints.

## Index

| Spec                                             | Domain(s)            | Status     |
|--------------------------------------------------|----------------------|------------|
| [events-workflow.md](events-workflow.md)         | events               | draft stub |
| [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)       | tmu                  | draft stub |
| [ace-support.md](ace-support.md)                 | ace, discord         | draft stub |
| [discord-integration.md](discord-integration.md) | discord, integration | draft stub |
| [flow.md](flow.md)                               | flow                 | draft stub |
| [sim-traffic.md](sim-traffic.md)                 | flow / stats         | draft stub |

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
