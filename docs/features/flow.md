# Flow (vatflow.io replacement)

## Problem

Replace [vatflow.io](https://github.com/djbrombizzle/vatflow) with a first-party traffic management / flow tool inside
OIS, feeding the TMU tooling (TMIs, average-delay page).

## Scope

To be scoped in Phase 1 after reviewing vatflow's feature set. Likely: flow programs / initiatives (define, publish), a
live traffic view, and the data feed that powers the
[average-delay page](tmu-ntml-adv-tmi.md).

## Data model *(placeholder)*

- `flow.programs` — id, name, params, `status` (draft→published), owner ARTCC.
- `flow.data` — ingested traffic/timing samples (source TBD; ties to sim-traffic).

## Permissions

- `flow.programs.{read,create,update,publish,delete}`
- `flow.data.read`

## API

Program CRUD + publish; read endpoints for the live view and the delay page.

## Open questions

- Full vatflow feature inventory — what carries over vs. drops.
- Traffic data source (see [sim-traffic.md](sim-traffic.md)).
- Relationship between a flow "program" and a TMU "TMI" — one publish, or linked?
