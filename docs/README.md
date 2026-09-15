# OIS documentation

In-repo documentation about OIS's code, architecture, and process. This is the **maintainer** side —
if you're looking for how to *use* the application, see the [user documentation site](../docs-site/)
(deployed at docs.&lt;domain&gt;) instead. Nothing here explains how to use a feature; everything here
explains how it's built.

## Architecture

- [architecture/overview.md](architecture/overview.md) — the services and how they fit together.
- [architecture/data-model.md](architecture/data-model.md) — Postgres schemas and key tables.
- [architecture/permissions.md](architecture/permissions.md) — the fine-grained permission model.
- [architecture/api-conventions.md](architecture/api-conventions.md) — REST shape, auth, errors, OpenAPI.
- [architecture/integrations.md](architecture/integrations.md) — VATSIM, VATUSA, Discord, email, service accounts.

## Operating OIS

- [deploy.md](deploy.md) — deploying to the test server / prod, the post-deploy health check,
  rolling back, and cutting a release.

## Feature specs

One spec per built subsystem — data model, exact permissions, API surface, Discord touchpoints. See
[features/README.md](features/README.md) for the index and the template for a new spec.

## Process

- [github-issues.md](github-issues.md) — how issues are labeled, structured, and moved across the
  board.

## History

[archive/](archive/) holds the project's pre-launch planning documents (the original proposal, the
phased build plan, an early ideas backlog) — kept for reference, not maintained as current. New ideas
belong in a GitHub issue, not a markdown file.

## Screenshot convention (for `docs-site`)

User-facing docs embed real screenshots under `docs-site/public/screenshots/`, named
`{section}-{page-slug}.png` (e.g. `map-facility-map.png`, `tmu-aadc.png`). Capture in light mode at a
1400px-wide viewport against mocked/seeded data (never real pilot/controller PII), one representative
"hero" image per page. Refresh a screenshot opportunistically when the page's UI changes meaningfully
— not on every text edit, and not on a schedule.
