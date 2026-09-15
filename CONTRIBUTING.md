# Contributing to OIS

Thanks for taking a look at OIS, VATUSA's operations platform. This file is the one-minute
orientation; the real detail lives elsewhere and this just points you at it.

## Where things live

- **[AGENTS.md](AGENTS.md)** — the source of truth: architecture, the permission model, `just`
  commands, testing, migrations, env vars, and the invariants that are easy to break by accident.
  Read this before making a non-trivial change.
- **[README.md](README.md)** — local setup (`just up`, `just backend`, `just web`).
- **[CLAUDE.md](CLAUDE.md)** — the thin agent-specific layer on top of `AGENTS.md`, for anyone
  (or anything) working on OIS with Claude Code.
- **[docs/](docs/README.md)** — architecture (`docs/architecture/`), per-feature specs
  (`docs/features/`), and deploying (`docs/deploy.md`).
- **[docs/github-issues.md](docs/github-issues.md)** — how issues, PRs, and the board work.

## Code of conduct

Be respectful and collaborative. Harassment, discrimination, or personal attacks toward anyone
in the project's spaces (issues, PRs, reviews, or anywhere else contributors interact) aren't
tolerated.

OIS is a VATUSA (VATSIM) community project — it's for flight-sim ATC operations, not real-world
aviation, and isn't affiliated with any real-world aviation authority. The VATSIM Code of Conduct
applies to conduct here.

## How to contribute

- **Filing a bug or feature request:** see [docs/github-issues.md](docs/github-issues.md) for the
  label taxonomy, body structure, and how issues move through the board.
- **Picking up work:** issues in **To Do** on
  [Project 7](https://github.com/orgs/VATUSA/projects/7/views/1) are cleared to start.
- **Before a large or architectural change:** open an issue to discuss the approach first, rather
  than sending a large PR cold.

## Dev workflow

The full branch model, `just` commands, and testing conventions are in AGENTS.md — see its
**Git workflow**, **Commands**, and **Testing & verification** sections. The short version:
work targets `next` (the integration branch); `main` is promoted from `next` separately and isn't
a target for issue work.

Two rules are easy to miss and fail silently if skipped — see AGENTS.md's **The API contract →
typed client** and **Permissions** sections:
- Regenerate the typed client after changing an endpoint or a `#[derive(ToSchema)]` model.
- A new permission or role touches three places that must stay in sync (AGENTS.md spells out
  exactly which).

## Pull requests

- Target `next`.
- Keep `just ci` green (fmt, clippy, tests, `pnpm lint`/`typecheck`) before requesting review.
- Keep the change scoped to the linked issue — don't fold in unrelated cleanup.
- Use conventional-commit-style messages (`type(scope): summary`) and a clear PR description
  (what changed, why, how you verified it).

## License

Contributions are accepted under the project's license — see the `LICENSE.md` file at the
repository root.

## Reporting a security issue

Please don't open a public issue for a security vulnerability. Use this repository's
[Security Advisories](https://github.com/VATUSA/OIS/security/advisories) (Security tab → "Report
a vulnerability") to report it privately.
