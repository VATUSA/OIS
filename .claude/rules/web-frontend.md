---
paths:
  - "web/**"
  - "packages/**"
---

# Web frontend

Loads when you read or edit `web/` or `packages/`. The web architecture (TanStack Router and
Query, data hooks in `web/src/lib/*` over the generated `ois` client, the settings registry,
realtime topic invalidation) is in `AGENTS.md` § Web and § Realtime. The generated client and its
regeneration are in § The API contract → typed client. The visual system is `DESIGN.md`, and UI
work goes through the shared shell and components in `packages/ui`. The web gates are in
§ Testing & verification. This file adds what has bitten web changes.

Sources: OIS lessons from #312, #336, #405, #477, #531, #539, #329, and #725.

## The contract is generated

Types for API data come from `@ois/api-client` (`packages/api-client`), never a hand-written
interface. If the type you need isn't there, the backend contract is missing it: change the
`ToSchema` model and regenerate. `pnpm typecheck` only tells the truth after that regeneration,
and turbo may replay a cached typecheck; use `pnpm typecheck --force` when it matters.

## Lint

`react-hooks/exhaustive-deps` is an **error** (`eslint.config.mjs:24`), not a warning. Fix the
dependency list. When a disable is genuinely right, a new one carries its reason on the same
line: `eslint-disable-next-line react-hooks/exhaustive-deps -- <why>`. The existing disables
predate this form; don't copy their reasonless shape.

## DOM tests

`jsdom` is installed in both `web` and `packages/ui`, with no shared DOM config: each file opts in
with `// @vitest-environment jsdom` on line 1 and is named `*.dom.test.tsx`
(`web/src/components/event-banner.dom.test.tsx`, `packages/ui/src/components/data-table.dom.test.tsx`).
Copy an existing one. "Untestable without a DOM" is not an excuse here: extract pure logic into
`web/src/lib/*` and cover the rest with a DOM test.

- **Seed the query cache; don't stub `fetch`.** `web/src/lib/api.ts:35` builds the `ois` client at
  module load, and openapi-fetch captures `fetch` then, so `vi.stubGlobal("fetch", …)` in a test
  arrives too late and does nothing. Mount the real component and seed data with
  `queryClient.setQueryData([...], value)`. To assert a request did or didn't happen, read the
  query cache: a query that never ran stays `status: "pending"`.
- **Seeded queries must not refetch.** Build the test client with `retry: false`,
  `staleTime: Infinity`, `refetchOnMount: false`, `refetchOnWindowFocus: false`, and
  `refetchOnReconnect: false`. Otherwise every seeded query refetches on mount, fails, and
  re-renders after your `act` block, racing the assertions.
- **A post-mount `setQueryData` needs a macrotask flush.** TanStack batches notifications outside
  React's scheduler, so `await act(async () => qc.setQueryData(k, v))` returns before the
  re-render. Use `await act(async () => { qc.setQueryData(k, v); await new Promise(r => setTimeout(r, 0)); })`.
  If the cache holds the new value and the DOM doesn't, this is the cause.
- **Don't fake a scoped user with `server_admin: true`.** `hasPermission` short-circuits on it
  (`web/src/lib/permissions.ts:11`), so a scoping test passes for the wrong reason. Use
  `server_admin: false` and a real permission tree.
- **Optimistic writes roll back with no backend.** A mutation that writes the cache optimistically,
  then POSTs, gets rolled back by `onError` when the POST fails. Dispatch in a synchronous `act`
  and read the cache immediately, and run a new DOM test in the full suite several times; this
  passed alone every time and failed about one run in two in the full suite.
- **Portals render into `document.body`.** Query `document`, not your mount node, and tear every
  mount down in `afterEach` (`root.unmount()`, remove the host, clear `document.body`), or the next
  test types into the previous test's portal. Stub `Element.prototype.scrollIntoView`, which jsdom
  lacks. `renderToStaticMarkup` cannot render anything that portals.

## Guards and wiring

A component test can't see a call site that drifts or a forbidden call that appears elsewhere.
Use a source-scan `*.guard.test.ts` for both; the method is in `test-quality.md` § Test the wiring
and § Absence needs a guard. The house examples are `web/src/components/*.guard.test.ts`,
`web/src/lib/*.guard.test.ts`, and `packages/ui/src/components/sector-grid.guard.test.ts`.

Never persist a credential in `localStorage`, `sessionStorage`, or a cookie set from script. A
token shown once (API keys, service-account tokens) is guarded that way (#531).

## A local repro needs the locked install

The installed `node_modules` drifts from `pnpm-lock.yaml`. Twice a map bug looked like a
version-skew regression because `@deck.gl/core` was installed at a newer version than
`@deck.gl/layers` (#477, #539), while the lockfile was correct. Before diagnosing any web bug
locally, and before believing a version-skew theory, run `pnpm install --frozen-lockfile` and say
in the issue which tree you diagnosed against.

## Realtime and caching

A mutation that changes flow or TMU state publishes a topic, and the web maps it to Query
invalidation (`AGENTS.md` § Realtime). When you add a query whose data a realtime topic changes,
add its key to that mapping, or the screen goes stale until the next poll.

Throttle an expensive key on every topic that invalidates it, not only on `feed.tick`. In
`web/src/lib/realtime.ts`, `FEED_KEYS` gives each key a `minGapMs` for the tick, but `TOPIC_KEYS`
invalidates at once, on every open client together. A frequent topic (`flow.release`, `flow.fca`)
mapped to a query whose request does real server work multiplies that work by viewers × open
queries. On #725 four flow topics refetched every open sector-demand table, each a fresh six-hour
projection on the server. Before mapping a topic to such a key, coalesce or throttle the refetch,
and say in a comment beside the key what one refetch costs the server.
