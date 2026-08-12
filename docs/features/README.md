# Feature specs

One spec per capability. Phase 1 fills these in before the matching backend domain is built. Each follows the template
below so a spec always states its data model, its **exact permission entries**, its API surface, and its Discord
touchpoints.

## Index

| Spec                                             | Domain(s)            | Status  |
|--------------------------------------------------|----------------------|---------|
| [access-control.md](access-control.md)           | access               | built   |
| [events-workflow.md](events-workflow.md)         | events               | spec    |
| [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)       | tmu                  | spec    |
| [ace-support.md](ace-support.md)                 | ace, discord         | spec    |
| [discord-integration.md](discord-integration.md) | discord, integration | spec    |
| [flow.md](flow.md)                               | flow                 | spec    |

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
