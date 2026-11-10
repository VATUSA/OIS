---
paths:
  - "**/*.test.ts"
  - "**/*.test.tsx"
  - "**/*_tests.rs"
  - "backend/src/scope_test_support.rs"
  - "web/src/test/**"
  - "fixtures/**"
  - ".config/nextest.toml"
  - "bin/lib/*.test.sh"
---
# Test Writing Rules (not loaded at startup; read in full before writing, changing or reviewing a test)

Companion to `test-quality.md`, which stays the always-loaded source for reading results, tiering,
the running stack and the mutation discipline. This file holds how to write tests: the narrow traps
first (each a way a test passes while the thing it is named for is broken), then the general
standards. None of it repeats `test-quality.md`.

Most OIS Rust tests live in a `#[cfg(test)] mod` inside the source file, which no `paths:` glob can
single out: read this file before writing one there too.

---

## Mutation traps

**A "writes nothing on failure" test whose injected failure fires before the write proves
nothing.** The write never got the chance to happen, so the test passes whether or not the guard
exists. Inject the failure after the write it guards.

**Restore a mutation by reversing the edit, never `git checkout -- <file>`.** Checkout restores
from the index and deletes unstaged implementation along with the mutation. Stage first if you
want checkout as an undo. Never mutate while a suite runs or builds in the checkout: it compiles
the mutant. `bin/mutate.sh FILE OLD NEW -- COMMAND...` applies one mutation, runs the command,
prints `RED` or `GREEN` and restores the bytes itself.

**A test named for a predicate must go red when that predicate is stubbed out.** Naming it is not
coverage: the function under test can return the expected value for an unrelated reason. Stub the
predicate once, run every test whose name claims it, and rename or delete each that survives.

**A table-driven test proves nothing about an entry a sibling entry already satisfies.** Pairing
every case with a principal that passes on its own proves nothing about the cases: a web test whose
user has `server_admin: true` passes every permission check, because `hasPermission` short-circuits
on it (`web/src/lib/permissions.ts`). Use `server_admin: false` and the real permission tree, drive
each entry on its own, then remove one entry and confirm only its case goes red.

**A guard that compares identities needs a same-size test.** "Refuse if the set of ids changed" is
proven only by swapping one member at the same count. A zero-to-one test still passes when the
comparison is weakened to `len()`; mutate it to `len()` and confirm the swap test goes red.

**Compound guards: isolate each clause, and re-check after simplifying.**
- To test one clause of `A && B && C`, start from a fixture that passes all of them and break only
  that field. A fixture that fails an earlier clause short-circuits, and the named clause can be
  deleted with the test green.
- `A || B` is the mirror: give every disjunct a fixture where it alone trips.
- **ANDing a new check onto a guard masks the old guard's tests.** The new check now refuses the
  same inputs, so the old mutations pass (#577). After adding a gate, re-run every earlier
  mutation, and give each half of the AND a fixture where the other half passes.
- When a guard is simplified, tests for the removed clauses go vacuous, not red. Grep the test file
  for each removed clause and delete what now duplicates a surviving-clause test.

**Mutate every predicate of a destructive `WHERE`.** List the predicates of a `DELETE` or `UPDATE`
and delete each in turn; each deletion must fail a test. Start the fixture from the most common
real row, then seed **one surviving neighbor per predicate** (same ARTCC and another sector; same
sector and another ARTCC). A `WHERE` tested against a one-row table proves nothing (#550, #656,
#706).

**A delta computed across loop iterations needs at least three iterations.** On the first the
previous value is zero, so `x += now - before` equals `x += now`. Pick the count where the wrong
arithmetic first crosses the asserted boundary.

**An `expect()` inside a `try` is swallowed by a `catch` below it**, because a failed vitest
assertion is a thrown error. The same holds for an `assert!` inside a closure run under
`catch_unwind`. Record the outcome in a variable and assert after the block, or narrow the catch.

**A guard test that rebuilds the guard's matching proves nothing.** Re-quoting the pattern under
test asserts a property of the test's own regex. Drive the real matcher with a subject shaped like
the bug. The same trap in expectation form: computing the expected value by calling the code under
test moves both sides together. Take the expected side from a literal or the requirement.

**A lexical guard must match the variable form of the call.** A scan catching
`localStorage.setItem("token", …)` misses `localStorage.setItem(key, …)`. Match the call shape,
prove no false positives across the swept directories, and prove red on a planted instance.

**When pinning a conditional by source text, assert the association, not the presence.** Separate
"A appears" and "B appears" checks pass with the branches swapped. Match the whole construct in
one expression. This applies wherever the suite cannot execute the code.

**A pattern that also matches the old value cannot prove the new one.** An assertion loose enough
to accept the pre-fix value stays green when the fix is reverted, so the property it is named for is
never tested. Assert the property that separates the fix from the bug, then revert the fix and
watch it go red.

## Fixtures that answer for the code

**A sort test is only real if the fixture arrives in the wrong order.** A small, freshly inserted
Postgres table usually returns rows in insertion order, which pre-sorts the fixture whatever the
code does. Insert fixtures in reverse of the expected order and mutate by deleting the sort.

**An assertion that reasons from position needs the query to order.** Without `ORDER BY`, Postgres
may return the passing order. Prefer adding the ordering to the production query; otherwise assert
on the set.

**A test of a comparison needs a fixture that flips it.** Size the fixture so correct behavior
gives the other verdict from the buggy one; a fixture where both give the same answer passes
against unfixed code.

**Test the far side of a boundary.** For `(a - b).abs() <= WINDOW`, a value just inside the window
passes with `.abs()` deleted, because a negative satisfies `<= n` on its own (#510). Assert that
both far sides reject and one inside accepts.

**Test a reset from the latched state, never from a fresh record.** A fixture whose flags start at
their defaults cannot see a value the code fails to clear, and a conditional reset needs one test
per branch.

**A fixture that fills a denormalized column hides code reading the wrong one.** A copy written by
a job or a sync step is null on rows created by paths that bypass it, but a hand-built fixture
fills it. Read from the source of truth, null the copy in the fixture, check real data first
(`select count(*) filter (where col is null) …` on the dev database, read-only), then point the
reader back at the copy and see red.

**Anchoring a rule on "the earliest related record" must prefer the live ones.** Exclude
cancelled, rejected and superseded children before taking the earliest; fall back to all only
when none are live. Test a stale-earliest case and an all-stale case.

**Search-exclusion tests pin every column the search touches.** When a test asserts a search
excludes a record, set every searched column on it to a literal; a generated default makes
exclusion probabilistic. The acting user can be a candidate too. Keep the term out of the positive
record's other searched columns, or one clause can be deleted with the test green.

**A test that writes the row directly cannot pin a guard in the code that writes it.** Inserting
the row with SQL and asserting a column did not change passes for every implementation. Drive the
real handler or repo function.

**Reproduce a defect through the API that produced it**, not a hand-inserted row. SQL inserts are
for the surrounding fixtures.

**Test an acceptance criterion through the entry point it names.** An AC about a route sends a
request through the real router (`scope_test_support::send`); an AC about a schedule drives the
`jobs.rs` pass (`*_once`) the job registry runs.

**Deleting or replacing a test: list every assertion it makes, and move each that no other test
makes.**

## Time

**Inject the clock whenever a test depends on time** (`test-quality.md` covers windows and
offsets). Two more shapes:
- A hardcoded future date is a scheduled break. Pass a fixed `now` and express fixtures relative
  to it.
- **A boundary the code snaps to a calendar** (the start of a day in UTC, an event's start time)
  needs a fixed mid-period instant. A fixture relative to the real clock crosses the boundary
  depending on when the suite runs, and fails for a whole period on unrelated branches.

## Framework quantities and fakes

**`web/src/test/no-network.ts` refuses every request but records nothing.** The generated client
(`web/src/lib/api.ts`, openapi-fetch) captures `fetch` when its module loads, so
`vi.stubGlobal("fetch", …)` in a test file arrives too late and does nothing to API calls. To
assert a request did or did not happen, read the query cache: a query that never ran sits at
`status: "pending"`. `fetchStatus` is useless, since it is `"idle"` either way once a request
settles.

**A count of enqueued jobs cannot see what the job does.** Asserting a row landed in
`integration.outbound_jobs` passes whatever its payload says. Assert the payload fields the bot
acts on, and the job that must not be enqueued.

**Verify any framework-internal quantity an assertion depends on.** A number you expect sqlx,
axum, TanStack Query or React to produce is a claim until you have printed it.

**A test can prove a vulnerability production lacks.** A forged `X-Forwarded-For` entry changes the
client address a test sees because no proxy appended a hop; in production the
`TRUSTED_PROXY_HOPS` chain decides (`backend/src/config.rs`, #588). Build the missing hops into the
request before believing an attack test.

**A mocked Tauri API call cannot fail on a missing capability.** The web tests `vi.mock` the
`@tauri-apps/*` modules, and at runtime a call the window's capability file does not grant
rejects, which a caller that catches it only logs. A new Tauri API call gets a guard test that its capability file
(`desktop/src-tauri/capabilities/*.json`) grants the permission to the windows that call it (#841).

**A source-scan test treats "cannot inspect" as a finding.** Count a file it cannot read as
failing, and add a count assertion that goes red if the glob stops matching. An assertion wrapped
in `if (anchor !== -1)` passes silently wherever the anchor is missing: assert the anchor first.

**Take both sides of a formatted-value assertion from the code.** A formatter can render two
candidate sources identically, so "contains no `d`" passes whichever source it read. Render the
expected string from each candidate source, assert they differ, then assert which one the output
matches.

**A show/hide pair needs both halves asserted**: `sr-only` with `focus:not-sr-only`, a `hidden`
attribute with what lifts it. Mutate each half.

## Security guards

**An extractor or check doing security work needs a test that names it.** Removing a
`RequirePermission<P>` argument, or the `permission_scope(...).allows(...)` call after it, leaves
ordinary tests green. Per gated route: a request without the permission is refused, a request
scoped to another ARTCC is refused, and the same request in scope succeeds. Then drop the guard and
see red.

**A guard a branch adds to close a leak gets a test written as the attack.** An ownership check,
a scope check on a path id, an authorize-before-validate reorder: one principal points the request
at another ARTCC's or another user's record and gets nothing back. Remove the guard and confirm
that test, not an unrelated one, goes red. A branch can close an IDOR issue with every test passing
whether or not the fix is there.

**A national-scope and a facility-scope principal are two paths.** A grant with `artcc_id` `NULL`
is national; one with an ARTCC is scoped. Test both, and a deny at one scope beside an allow at the
other (deny beats allow).

## vitest + jsdom

A DOM test opts in with `// @vitest-environment jsdom` on line 1; `packages/ui` and `web` both carry
`jsdom`. Mount the component for real and feed it data through the query cache. Each of these has
already cost a false start:

- **Seed the query cache, never stub `fetch` or mock the data hooks.** Build a `QueryClient`,
  `setQueryData` the keys the hooks read, and mock only `@tanstack/react-router` where navigation is
  needed. Seeded queries must not refetch, or a failed request re-renders after your `act`: build
  the client with `retry: false`, `staleTime: Infinity`, `refetchOnMount: false`,
  `refetchOnWindowFocus: false` and `refetchOnReconnect: false`.
- **A post-mount `setQueryData` needs a macrotask flush, not just `act`** (#405). TanStack batches
  cache notifications outside React's scheduler, so `await act(async () => qc.setQueryData(k, v))`
  returns before the re-render. Use
  `await act(async () => { qc.setQueryData(k, v); await new Promise((r) => setTimeout(r, 0)); })`.
  If the cache is right and the DOM is stale, it is this.
- **Tear every mount down in `afterEach`** (`root.unmount()`, `host.remove()`,
  `document.body.innerHTML = ""`). `Modal` portals to `document.body`, so a mount left standing is
  still in the document and the next test's query hits it (#312, #336). Query against `document`,
  not the node you mounted into.
- **An optimistic write rolls back.** With no backend the mutation rejects and `onError` restores
  the cache, so `await act(...)` before reading races the rollback. Dispatch in a synchronous `act`
  and read the cache immediately, and run a new DOM test against the full package suite several
  times: it can pass alone and fail one run in two beside the rest.
- jsdom has no layout engine: stub `Element.prototype.scrollIntoView = () => {}` where the component
  scrolls, and leave geometry to the running stack (`test-quality.md` → "jsdom vs the running stack
  — the decision rule").

## Test the wiring, not just the unit

The regression in OIS is usually a call site drifting, not the unit misbehaving: reverting the real
bug has left the suite green when the unit was tested and its caller was not (#428, #433, #419).
Four shapes have precedent:

- **A source scan over call sites**, named `*.guard.test.ts` (`web/src/components/shell/window-chrome.guard.test.ts`,
  `web/src/lib/api-paths.guard.test.ts`).
- **A handler-level `#[sqlx::test]`.** If the extractors get in the way, split the handler into a
  thin `get_x` and an `x(pool, …)` that holds the logic, and still prove the gate through the router
  (below). Never add a test-only constructor to a permission type.
- **A real request through the real router** with `scope_test_support::send`, which returns the
  status, or `send_json`, which also returns the body when the test must check what the handler
  returned.
- **A test on the `jobs.rs` pass itself** when a background job is the writer. The private `*_once`
  functions are the recurring blind spot; `mod ace_reminder_tests` in `backend/src/jobs.rs` is the
  precedent.

## Absence needs a guard, not a grep

An acceptance criterion that a side effect is **absent** ("nothing persists the token client-side",
"never calls the network") cannot be verified by grepping once; that proves today, not tomorrow. On
#531 the grep was clean, and a reviewer then added a `localStorage.setItem` of the token with all
tests green. Pin it with a `*.guard.test.ts` source scan:

- scan every file the value flows through, not just the component
- strip comments first, because prose that mentions the forbidden call is not a call
- include a negative case, or a matcher that stops matching passes forever
- prove it by injecting the forbidden call into a **new throwaway file**, never a real one
- say in a comment why it is a source scan, so nobody swaps it for something weaker

## Testing Pyramid
- **Unit:** pure logic with no database, in a `#[cfg(test)] mod` or a node-environment vitest file.
  The trajectory model, the permission tree, metering and parsers live here.
- **Database:** `#[sqlx::test]` for repo logic, `jobs.rs` passes and anything whose correctness is
  in SQL.
- **Router:** `scope_test_support::send` / `send_json` for every gate, every route an AC names, and
  what a handler returns.
- **DOM:** jsdom vitest for component interaction.
- **Running stack:** the page or endpoint exercised against a throwaway database, for what no layer
  above can see.

## Coverage Targets
- No coverage tool runs in the OIS gate, so no percentage is enforced or claimed.
- Every branch of a new conditional gets a test that reaches it; branch coverage matters more than
  line coverage for conditionals.
- New code gets its tests in the same change (write tests alongside implementation).

## FIRST Principles
- **Fast:** a unit test runs in milliseconds; keep the database for what needs it.
- **Isolated:** no test depends on another. `#[sqlx::test]` gives each its own database; a jsdom
  test tears its mounts down.
- **Repeatable:** never hit real external APIs (VATSIM, VATUSA, Open-Meteo, AWC, Discord). Inject
  the clock; never `set_var`.
- **Self-validating:** assert specific values; never rely on manual inspection.
- **Timely:** write alongside implementation or before it.

## Test Naming
```rust
#[sqlx::test]
async fn update_refuses_a_principal_scoped_to_another_artcc(pool: PgPool) { /* … */ }
```
```ts
it("keeps the palette open when a scope chip is clicked", () => { /* … */ });
```
The name states the behavior and its condition, and the body asserts every clause of it
(`test-quality.md` → "Green proves nothing on its own").

## STUPID Tests (delete or rewrite)
- Mock-returning-mock (tests the mock, not the code)
- Tautology (`assert_eq!(x, x)`)
- Rendering-only (a status `200` or a render without throwing as the ONLY test for a form)
- Assertion-free (runs code but never asserts)
- Implementation-detail (breaks on refactor, feature still works)
- Constructor/getter tests
- Over-mocked (everything mocked, nothing real tested)

## VALID Tests (keep)
- Behavior verification (input → output/side effect)
- Integration (multiple components working together)
- Edge cases (null, zero, empty, boundary values)
- Regression (scenario that broke in production)
- Authorization (a principal without the permission, or out of scope, is refused)

## Five Questions Before Keeping a Test
1. Would this fail if I introduced a bug?
2. Does this test behavior or implementation?
3. Is this testing MY code, not the framework?
4. Could I delete the implementation and this still passes?
5. Is there another test covering the exact same scenario?
If ANY answer is wrong — rewrite or delete the test.

## Test a parser against captured bytes, never against input you typed

When the thing under test reads text some *other* program produced (a hook payload parser, a
regex over command output, an NTML or advisory document, a VATSIM feed record), the sample must be
captured from that program and kept. Type the sample yourself and you have written both halves of
the test from one mental model of the format, so it passes because the two halves agree with each
other, and it will keep passing while the real thing fails. The reference documents in
`fixtures/*-reference.json`, which `backend/src/advisory.rs` asserts against, are the pattern.

Colored output is the standing trap: a summary line on the wire carries ANSI escapes the terminal
hides, and `trim()` removes whitespace, not escape sequences. The discipline is three commands:

```bash
cargo nextest run -p <crate> -E 'test(<name>)' > "$SCRATCH/sample.txt" 2>&1   # capture
grep -c $'\e' "$SCRATCH/sample.txt"                                            # look at the bytes
bash <parser> < "$SCRATCH/sample.txt"                                          # drive the parser
```

**Look at the bytes before you write the pattern.** ANSI escapes, `\r`, BOMs, non-breaking
spaces and trailing whitespace are all invisible in a terminal and all break a naive match.
`cat -v`, `xxd | head`, or a grep for `\e` settles it in one command; reasoning about it settles
nothing.

**Drive every branch from real samples too, not just the happy one.** Keep a green capture and a
red capture. A parser that only ever sees success cannot be shown to fail closed, and failing
closed is usually the property that matters.

**Only `just test` (`bin/run-tests.sh` with no arguments) writes the tests marker**, from the gate's
own process, so a backgrounded or redirected run counts. It stamps only a full run in which every
step passed, on a tree that was clean at the start and whose HEAD and tree did not move during the
run; `--no-stamp` (`just ci-full`) and any targeted argument never stamp. Writing the marker
yourself is prohibited (CLAUDE.md → "Absolute Prohibitions (enforced by hooks)"). Run it alone on
the line, or as exactly `cd /absolute/checkout/root && just test` (`bin/review-marker.sh KIND SHA`
takes the same two forms; a redirect to a literal path
outside `.claude/` and `.git` is fine on either; a `$VAR`, `~` or `tee` target does not stamp): the
marker ledger keeps a manifest line only from a call that is nothing but a stamper and reads, or a
backgrounded one's grant, and undoes any other (`.claude/hooks/lib/marker_ledger.py`). That guards
against a mistake or a shortcut; a deliberate multi-step forgery of the ledger is outside it. See
`test-quality.md` → "A gate counts once per SHA".

**When the guard path and the failure path return the same value, the test cannot tell them
apart.** A short-circuit returning the caller's default is indistinguishable from a call that
ran, failed and fell back to that same default. This is the exact shape of the
unconfigured-integration short-circuit: a VATUSA client returning nothing because
`VATUSA_API_KEY` is unset looks identical to one returning nothing after the API failed. Assert a
side effect that differs (a log line, a row, a call that did not happen) or move the assertion to
the predicate itself.

**The test harness can make a code path reachable that production shadows.** A test can be green,
survive mutation, and still pin a response that cannot occur in production, because something runs
before the code under test there and not here. Before trusting a test of an error path, ask what
runs first in production (the rate limiter, the audit middleware, the reverse proxy) and whether it
shares the dependency being failed. Where the path really is shadowed, say so in the test's doc
comment rather than deleting it: the assertion still pins the handler's contract.

**Forcing a failure with DDL is contained only inside `#[sqlx::test]`.** Postgres DDL is
transactional and each `#[sqlx::test]` owns its database, so a `drop table` there touches nothing
else. Never run DDL against the shared dev database to force a failure; prefer an injected error
anyway.

## What to ALWAYS Test
- Happy path, validation (invalid input rejected with `400 bad_request`), authorization
- Edge cases: null values, empty collections, zero amounts, boundary values
- Error paths: external service failure (the VATSIM feed, VATUSA, Open-Meteo, AWC, the bot's lease
  and ack), missing data
- Database state: rows created/updated/deleted correctly
- Side effects: an outbound job enqueued, a realtime topic published, an audit row written

## What to NEVER Test
- Framework internals (axum routing, the TanStack Query cache, sqlx pooling)
- Getter/setter with no logic
- Constructor field assignment
- Third-party library behavior

## Mocking
- Mock externals at their boundary; NEVER mock the thing you're testing
- Rust: the feed and clients are structured so the pure logic is testable without the network; test
  the logic, not a mocked HTTP layer
- Web: seed the query cache; mock only `@tanstack/react-router`; never stub `fetch`
- Trace the FULL call path before mocking: read the function AND everything it calls
- NEVER hit real external APIs in tests

## Fixture Rules
- ALWAYS set the fields that matter for the test assertion, in the test body
- NEVER rely on a helper's or a column's default for a test assertion
- Use `scope_test_support` (`seed_user`, `grant`, `deny_scoped`, `session_cookie`, `test_state`,
  `artcc`) before writing your own setup

## Endpoint and page tests (MANDATORY per route and page)
1. A new or changed route: a router-level test of the success response, the refusal without the
   permission, the refusal out of scope where the route is scoped, and invalid input
2. A new or changed page: a render test, and an interaction test for every control it adds
3. Additional: edge cases, and the running-stack walk `test-quality.md` requires

### A handler call cannot prove an authorization gate

`RequirePermission<P>` has a private field, so a gated handler cannot be called directly. Every
workaround tests something else. A test of the split-out `x(pool, …)` runs below the extractor, and
a test of `can_edit`/`require_edit` alone stays green when a handler stops calling it (#198, #364).
**Neither is evidence that the route is protected**, and reading it that way is how the gap survives
review.

To test the real gate, send the request through the real router:

```rust
#[sqlx::test]
async fn update_refuses_a_principal_scoped_to_another_artcc(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::from([("ZDC".into(), artcc(&["KIAD"]))]));
    let user = seed_user(&pool).await;
    grant(&pool, &user, "flow.sector_limits.update", Some("ZNY")).await;
    let cookie = session_cookie(&pool, &user).await;

    let status = send(&state, Method::PUT, "/api/v1/…/ZDC/…", &cookie, Some(json!({ … }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // positive control: the same principal granted at ZDC succeeds; the row is unchanged here
}
```

Three things that make this probe lie:

- **Each gate answers with its own status.** A missing permission is `401`, a wrong facility `403`
  (`scope_test_support::send`'s doc comment). Assert the exact code, or a test of the scope check
  passes on the permission check.
- **Assert on the record, not only the status code.** Read the row back: a refusal that wrote
  anyway is the bug.
- **Keep a positive control** in the same test: the same request from an in-scope principal
  succeeds. Without it, a green result is equally explained by a setup that never reached the
  handler.

## No Helper Methods in Tests
- DRY applies to request plumbing, not to what a test asserts. Shared setup lives in
  `scope_test_support.rs`; a per-file helper may build requests.
- The fixture values and expected values an assertion depends on stay visible in the test body.
- Repeat code across tests when it makes a test clearer.

## Backdating `updated_at` under the touch trigger

Tables with `platform.touch_updated_at()` run it `before update`, and it sets `updated_at = now()`.
An `update … set updated_at = now() - interval '2 days'` on such a table is overwritten by the
trigger, so a fixture meant to look stale is fresh and a retention or staleness test passes for the
wrong reason. Set the backdated value in the `insert`, which the trigger does not touch, and read
the row back to confirm the value landed.

## Feed-visible config tests must reload the cache

The feed has no database handle: it reads `AppState` `ArcSwap` caches that a `jobs.rs` refresh or a
write handler's force-reload fills. A test that inserts a config row (an aircraft profile, say) and
then calls feed code reads the cache as it was, not the row. Drive the write through the handler
that force-reloads, or reload the cache explicitly, and assert the feed output changed.

## Test environment mutation needs a paired reset

Better: don't mutate the environment at all. `std::env::set_var` is process-global and races the
rest of a `cargo test` run. OIS passes configuration explicitly instead: a field on the `AppState`
the test builds (`metrics_token`, `backend/src/state.rs`) or an explicit argument
(`reconcile_login_access` takes the admin list, `backend/src/handlers/auth.rs`). Add that seam rather
than reaching for `set_var`. A web test that must change a global restores it in `afterEach`.

## Test templates

**`jobs.rs` pass:**
```rust
#[sqlx::test]
async fn a_claim_due_without_discord_still_nudges_clients(pool: PgPool) {
    // seed rows with SQL, anchored on now()
    let events = crate::realtime::Events::new(None);
    let mut received = events.subscribe();
    ace_reminder_scheduler_once(&pool, &events).await.unwrap();
    assert_eq!(received.try_recv().expect("a nudge was published").topic, topic::EVENT_REMINDER);
}
```

**Router-level read:**
```rust
let (status, body) = send_json(&state, Method::GET, &uri, &cookie).await;
assert_eq!(status, StatusCode::OK, "{uri}: {body}");
assert_eq!(body["rows"][0]["limit"], 12);
```

**jsdom interaction:**
```tsx
// @vitest-environment jsdom
it("marks the chip pressed when clicked", async () => {
  const qc = seededClient();            // retry: false, staleTime: Infinity, no refetch
  qc.setQueryData(["me"], me);
  await act(async () => root.render(<QueryClientProvider client={qc}><Widget /></QueryClientProvider>));
  document.querySelector<HTMLButtonElement>("[data-chip]")!.click();
  expect(document.querySelector("[data-chip]")!.getAttribute("aria-pressed")).toBe("true");
});
```
