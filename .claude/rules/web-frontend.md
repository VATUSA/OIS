---
paths:
  - "web/**"
  - "packages/**"
  - "desktop/package.json"
  - "eslint.config.mjs"
---
# Web Frontend Rules (not loaded at startup; read in full before writing or reviewing TypeScript or React)

The web architecture (TanStack Router and Query, data hooks in `web/src/lib/*` over the generated
`ois` client, the settings registry, realtime invalidation) is `AGENTS.md` § Web and § Realtime; the
generated client is § The API contract → typed client. The visual system is `DESIGN.md`. Test
mechanics (jsdom, seeding the query cache, source-scan guards) are `test-writing.md`; when to use
jsdom and when the running stack is `test-quality.md` → "jsdom vs the running stack — the decision
rule". This file is the TypeScript and React layer on top.

---

## The contract is generated

Types for API data come from `@ois/api-client` (`packages/api-client`), never a hand-written
interface. If the type you need isn't there, the backend contract is missing it: change the
`ToSchema` model and regenerate (`rust-backend.md` → "The contract"). `pnpm typecheck` only tells the
truth after that regeneration, and turbo may replay a cached typecheck from another worktree; use
`pnpm typecheck --force` when it matters.

## Data and state
- Server data goes through a hook in `web/src/lib/*` over the `ois` client and TanStack Query. A
  component never calls `fetch` and never keeps a second copy of server data in `useState`.
- A user-facing setting is an entry in `web/src/lib/settings/registry.ts`, read with
  `useSetting(key, default).value`; it then appears on `/settings` with no other wiring.
- `hasPermission` (`web/src/lib/permissions.ts`) mirrors the backend for display only. Hiding a
  control is not authorization; the backend refuses the request regardless (`secure-coding.md`).

## Realtime and caching

A mutation that changes flow or TMU state publishes a topic, and the web maps it to Query invalidation
in `web/src/lib/realtime.ts` (`AGENTS.md` § Realtime). When you add a query whose data a topic
changes, add its key to that mapping, or the screen goes stale until the 60 s fallback poll.
`web/src/lib/realtime-keys.guard.test.ts` refuses a dead entry (#647); nothing catches a missing one.
A feed-derived query polls only while feed ticks are not arriving, through `pollUnlessLive`
(`web/src/lib/feed-polling.guard.test.ts`, #648).

## UI goes through the shared system

Any screen, component, color or spacing change follows `DESIGN.md`: one dark operator-console system,
one accent, tokens only. Restyle through the shell and shared components in `packages/ui`
(`@ois/ui`); never hand-style a one-off screen.

## Lint

`react-hooks/exhaustive-deps` is an **error** (`eslint.config.mjs`), not a warning, since #329. Fix
the dependency list. When leaving a dependency out is genuinely right, the disable carries its reason
on the same line: `// eslint-disable-next-line react-hooks/exhaustive-deps -- <why>`. Older disables
without a reason are not a pattern to copy.

## Credentials in the browser

Never persist a credential in `localStorage`, `sessionStorage` or a cookie set from script. A token
shown once (API keys, service-account tokens) is guarded that way by a source scan (#531). The web
session is the `HttpOnly` cookie; the desktop token lives in the OS credential store and reaches the
client through the token provider in `web/src/lib/api.ts`.

## A local repro needs the locked install

The installed `node_modules` drifts from `pnpm-lock.yaml`. Twice a map bug looked like a version-skew
regression because `@deck.gl/core` was installed at a newer version than `@deck.gl/layers` (#477,
#539), while the lockfile was correct. Before diagnosing any web bug locally, and before believing a
version-skew theory, run `pnpm install --frozen-lockfile` and say in the issue which tree you
diagnosed against.
