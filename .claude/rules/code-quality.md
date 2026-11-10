# Code Quality Rules (always loaded)

---

## SOLID
- **Handlers (`backend/src/handlers/`):** HTTP concerns only: extractors (permission, principal,
  path, body), validation, delegate to a repo, respond. No SQL.
- **Repos (`backend/src/repos/`):** every query, binding every value; DB failures map to `ApiError`.
- **Models (`backend/src/models/`):** request/response and row types (serde, utoipa, `sqlx::FromRow`).
- **Feed (`backend/src/feed/`):** computation over the `AppState` caches; the prediction and metering
  code takes no DB handle (`feed-and-trajectory.md`).
- **Jobs (`backend/src/jobs.rs`):** a thin `spawn_*` loop around a `*_once(pool, …)` pass that a test
  can call.
- **Web:** pages compose; data hooks in `web/src/lib/*` own queries and mutations; shared components
  live in `packages/ui` (`DESIGN.md`).
- NEVER put business logic in a handler or a React component. A handler that grows logic splits into
  a thin `get_x` holding the extractors and an `x(pool, …)` holding the logic.
- NEVER create a God module. Split by domain.

## Unjustified scope

Ship what was asked for and nothing else. A module, endpoint, permission, config flag, job, migration
or route that no requirement calls for is a **MAJOR review finding** and comes back out of the diff
before merge, however small and however useful it might one day be.

The test is whether you can name the requirement it serves. "It seemed useful", "it might be needed
later" and "it was quicker to generalize" are not requirements. A `jobs.rs` pass nothing needs is the
canonical case: it carries a schedule someone has to reason about on every replica, a failure mode
nobody monitors and a test nobody wrote, in exchange for a capability nobody requested.

Two things are in scope by definition: a fix to a defect this branch itself introduced, and work the
user approved in the session. Everything else outside the issue's logical scope gets left alone and
named in the report; see `github-issues.md` → "Scope, and when a follow-up issue is allowed".

This is the mirror of CLAUDE.md rule 7. Rule 7 stops you shipping a feature that is only half-wired;
this stops you shipping a capability nobody asked for.

## Complexity Limits
- Function length: < 40 lines
- Nesting depth: < 3 levels
- Module: < 500 lines unless it is one concern
- Parameters per function: < 5; clippy's `too_many_arguments` fails the build above 7, so group
  related parameters into a struct rather than `#[allow]` it
- PR size: < 600 lines when it splits cleanly

Only clippy's limit is a gate. Over any other line, propose a split or say why the code still reads
clearly; in review it is a SUGGESTION, never a blocker.

## Static Analysis
- `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm typecheck` and `pnpm lint` are the
  static gates; CI runs all three. `cargo fmt --all -- --check` and ESLint/Prettier own style.
- New code passes clean. There is no baseline file: a warning is fixed, not recorded.
- NEVER remove or disable a check to make it pass: no `#[allow(...)]`, `eslint-disable`,
  `@ts-ignore` or `@ts-expect-error` without the reason on the same line, no deleted test, no
  weakened assertion, no `--no-verify`. The one exception is a *confirmed pre-existing* failure:
  reproduce it on `origin/next` first and say so in the PR (`test-quality.md`).
- `pnpm typecheck` tells the truth only after the client is regenerated for a contract change
  (`AGENTS.md` § The API contract → typed client).

## Exception Handling
- Propagate with `?`. A `match` or `map_err` that discards an `Err` must log it **and** surface the
  failure in the return value (see Fail-Open below).
- NEVER swallow: `let _ = fallible().await;`, `.ok()` that drops an `Err`, `unwrap_or_default()` on
  an I/O or DB result.
- NEVER `unwrap()`/`expect()` on request input, a DB result or an external response in non-test code.
  `expect` is for an invariant the code itself guarantees, with the invariant as its message.
- Handle: external API failures (VATSIM, VATUSA, AWC METAR, Open-Meteo winds, Discord), expected
  business errors, missing optional data. Propagate: database failures, config errors.
- Users see the stable error envelope (`AGENTS.md` § Conventions & gotchas), never a SQL error, a
  stack trace or a type name.
- Separate failure semantics get separate handling. A follow-up (a realtime publish, a warning) runs
  after the work, so its failure neither undoes the work nor goes unreported.
- **A handled error loses the signal a panic gave.** `map_err(|_| ApiError::Internal)` drops the
  cause, and the request log (`reqlog.rs`) records only the 500. Where the cause matters for
  diagnosis, log it first with `tracing::error!`, within the Logging rules of `secure-coding.md`.

## Fail-Open / Silent Success

An operation must never report success on a path that failed.
- A `jobs.rs` pass that catches, logs and returns `Ok` is invisible to the job registry and to
  `/metrics`. Return `Err(detail)` so `run_interval` records the run as failed; a pass with several
  independent steps finishes them all, then returns `Err` if any failed
  (`ace_reminder_scheduler_once` is the pattern).
- **A silent skip is the same bug.** `if let Some(x) = lookup { … }` with no `else` turns a
  misconfiguration into nothing happening. On #436 a renamed channel constant left every stored
  mapping resolving to `None`; the publish skipped the enqueue, returned 200 and logged nothing. Give
  the `else` a `tracing::warn!` at minimum (`skipped_post` in `handlers/tmu.rs`).
- Validation must **fail closed**: an unrecognized, unverifiable or unparseable value is REJECTED,
  not passed through. Scope checks follow the same rule: `PermissionScope::allows(None)` is true only
  for unrestricted national scope.
- A write or I/O helper must not return `Ok` (or a URL, or `true`) when the underlying call failed.
- **Never decide whether an operation failed by asking the same subsystem a second question.** Act,
  then trust that call's result: `rows_affected()`, the `returning` row, `fetch_optional`'s `None`.
- **Looping over records each changed irreversibly: handle the error inside the loop**, collect the
  ids, finish the batch, and report which are outstanding and what did succeed. An operator who sees
  only "failed" retries against records already changed.
- **Acquire the resource last.** A session-level `pg_advisory_lock` on a pooled connection outlives
  the request that took it: dropped mid-reply, the connection goes back to the pool still holding the
  lock. `repos/access_reset.rs` detaches the connection before the attempt for that reason. Validate
  first; release on every path.
- **An invariant enforced in one write path covers only that path.** The audit middleware
  (`audit.rs`) records HTTP mutations only; a handler's force-reload refreshes only its own replica's
  cache. A branch that adds such an invariant greps every writer of the guarded rows (handlers,
  `jobs.rs` passes, the feed collectors, the bot's ack path, migrations) and routes or tests each.
  The migration case is in `database-postgres.md` → "A data migration bypasses every handler side
  effect, and the caches they reload".
- **A key for a write that is never overwritten (a date, a cycle, an event window) is parsed
  strictly**: exact format, checked by round-trip, nothing outside the range the write may record.
- **A guard keyed on a value refuses every cause that shares it, not the one you meant.** On #287 a
  `NULL` pushback meant both "this departure did not push" and "the push was not detected"; counting
  every `NULL` as zero taught a hub a 0 s pushback. Before a predicate stands in for a cause, list
  every way the value is reached, store the causes apart where you can, and write one test per cause.
- **A "nothing dropped silently" check covers every level the data is grouped into.** Code that
  builds parents from child rows loses a childless parent at each level. One report and one test per
  level, each seeding a childless record beside a sibling that has children.
- **A hook gating a shell command parses its arguments; it never greps for a token.** A substring
  check passes a second refspec, a chained `;` command and a `#` comment. Refuse separators, parse
  options and refspecs, and test each bypass shape (`.claude/hooks/` and `bin/pre-push` are the
  standing cases).
- **Sanctioned exception: a deliberately disabled integration is not fail-open.** With
  `VATUSA_API_KEY` unset, `config::vatusa_api_key()` is `None` and sync no-ops; with no
  `OIS_SECRET_KEY`, webhook registration is off. That is intent, not failure. A *configured*
  integration whose call failed must surface it. Don't "fix" the unconfigured short-circuit into an
  error.

## Logging
- `tracing::debug!`: development flow, variable inspection.
- `info!`: successful operations, milestones, state changes.
- `warn!`: ONLY if all three: actionable + a real problem + needs watching (a misconfiguration, a
  skipped side effect, a failed external call).
- `error!`: genuine failures needing attention. `reqlog.rs` already logs every 5xx at ERROR and every
  4xx at WARN; don't log the same request twice.
- ALWAYS structured fields with ids: `tracing::warn!(tmi_id = %id, channel, "…")`.
- What must never be logged: `secure-coding.md` → "Logging".

## Caching

OIS has no shared cache store. Its caches are in-process, per replica:

- **Feed-visible config** (aircraft profiles, sector limits and consolidations, gates, flight
  exclusions, taxi samples, airspace sectors, nav, winds): an `AppState` field behind `ArcSwap`,
  loaded by a `jobs.rs` refresh pass, and **force-reloaded by the write handler** so an edit applies
  at once. A new feed-visible config mirrors that pattern: cache + refresh job + force-reload on
  write. The feed reads the cache and never queries.
- **Force-reload reaches one replica.** Every other replica sees the write at its next refresh tick
  (`SECTOR_LIMITS_INTERVAL` is 30 s, `AIRCRAFT_PROFILES_INTERVAL` 5 min). A writer that is not the
  handler (a migration, a job, `psql`) reaches every replica only at the tick. Size the interval for
  how stale the surface may be.
- **A derived value is keyed on every input it reads.** `handlers/sector_demand_cache.rs` (#725) and
  the feed's `TrafficCache` (#588) compare each input by identity (`Arc::ptr_eq` on the table a reload
  replaces) or by value; an entry is reused only while all of them match. Before caching a derived
  value, list every input, give each one a key or an invalidation, and write one test per input:
  change it, assert the value refreshes, then drop that key and watch the test go red.
- **A read-through cache has one TTL, shared.** `handlers::runway::metar_for` holds METAR for
  `METAR_TTL_MS` (10 min), and `airport_configs::wind_for` uses the same cache rather than a second
  one with its own idea of stale (#510).
- **An in-process map keyed by request input must be bounded.** `ReplayGuard` forgets bodies older
  than its window on every insert; the rate limiter calls `retain_recent`. A map that only grows is a
  memory leak per replica.
- **The web caches through TanStack Query**, invalidated by realtime topics (`web/src/lib/realtime.ts`,
  60 s fallback poll when the socket drops). A mutation that changes flow or TMU state publishes its
  topic after commit (`AGENTS.md` § Realtime).
- NEVER hold mutable state in a `static` or a `thread_local!` as a cache. It belongs in `AppState`,
  where tests build it fresh (`scope_test_support::test_state`).

## A visibility filter leaks back through every path you did not change

Adding a filter at one read path fixes that path and nothing else. There are always more routes to
the same record than the one that reported: a second repo function, a public route, a debug view, a
write that resolves its target through its own lookup. On #586 `list_fcas` hid unpublished and
deleted FCAs, while the public traffic route loaded through `get_fca`, which applied neither filter,
so anyone holding an FCA's UUID read its metered traffic with no credential. On #746 the order,
release and swap routes skipped the event-FCA guard the edit routes had.

Fix it where every path resolves the record: one repo function or one guard
(`require_visible_event_fca`) that every route calls. Before shipping a filter, grep every query
and every route that reads the same table and confirm each goes through the filtered path.

## Performance
- Zero N+1 queries: no query per row in a loop. `fetch_api_key_access` once ran two queries per key
  permission inside its loop (#543); one owner resolution replaced them.
- Index foreign-key columns and columns in `where`, `order by` and joins. Postgres does not index a
  referencing column for you.
- CPU-heavy work (route resolution, metering, projection) runs under `tokio::task::spawn_blocking`,
  never inline on an async worker and never `block_in_place`, which still ties up the worker (#63).
- Clone the `Arc<Snapshot>` and drop the feed lock before the CPU work (`FeedInner::snapshot`).
- Stream a large result with `.fetch(pool)` instead of `fetch_all` (`repos/stats.rs` taxi samples,
  #776).
- **No synchronous third-party call on a request a user is waiting for** unless the response needs
  its answer. A Discord post is an outbound job, never a call from the handler.

## External API clients must tolerate absent credentials

Integration credentials are optional by design: `.env.example` and tests carry no real VATSIM,
VATUSA or Discord keys.

1. Read the credential through a `config.rs` getter that returns `Option`, treating blank as unset
   (`trimmed_env`).
2. On `None`, short-circuit before any network call and say so once; do not error and do not panic.
3. Never paper over it with a fake key in a test env. Tests never reach a real external API
   (`AGENTS.md` § Testing & verification).

This is the "sanctioned exception" in Fail-Open above, stated as a construction rule.

## Queues and transactions
- **Enqueue an outbound job inside the transaction of the change that causes it.**
  `integration_repo::enqueue_job` takes the caller's `&mut Transaction`, so there is no job without
  the change and no change without the job. The queue is a table in the same database, which is why
  this is right here.
- **Publish realtime topics and start follow-up tasks after `commit()`.** A topic before the commit
  sends every client to refetch the old state (`handlers/ace.rs` publishes after commit).
- Resolve lookups that can fail or that need no lock before `pool.begin()` (`handlers/tmu.rs`
  resolves the NTML channel first).
- **A retry does not re-run every step, and a late ack is not this worker's ack.** The reaper
  returns stranded `in_progress` jobs to `pending` (#446); a reaped worker's late success would mark
  a re-queued job delivered. `ack_job` fences on status and on the lease's `attempt` (#472). Trace
  the retry path before relying on it.
- **Select-then-insert is a race across replicas.** Dedup with a unique index and
  `on conflict do nothing` (0066 for ACE reminders), or `for update skip locked` (`lease_jobs`).
- **A parent row and its children commit together**, or the parent is removed when no child was
  created. Test a failure on the first child and on a later one.

## Background passes (`jobs.rs`)
- **Every replica runs every pass**; there is no leader. A pass with a side effect is idempotent
  across replicas: a unique index, `for update skip locked`, or a transaction-scoped advisory lock
  (`pg_advisory_xact_lock`) keyed on the thing it guards.
- A pass returns `Err(detail)` on failure (Fail-Open above) and is split into a `*_once` a test calls
  directly (`test-writing.md` → "Test the wiring, not just the unit").
- **Size the interval against what reads the result.** A refresh pass's interval is the staleness
  every other replica sees; a prune pass's retention constant is a policy someone chose. A changed
  interval or retention is a behavior change, named in the PR.
- A pass that acts for a user carries that user's principal and checks it at run time
  (`secure-coding.md` → "A rule that preserves access must check the access still exists").

## Recurring alert dedup

A pass that finds the same row on every run MUST dedup per entity, or one row produces an alert per
run until someone mutes the channel. The ACE reminder pass is the pattern: `claims_due_for_reminder`
excludes claims already reminded (`not exists` against `integration.outbound_jobs`), and the partial
unique index from migration 0066 makes the insert itself the dedup, so two replicas that both see a
due claim send one DM. Key the dedup on the entity and the alert kind.

## Database Migrations
- Forward-only: a new numbered file, never an edit to an applied one.
- Index columns used in `where`, `order by` and joins.
- See `database-postgres.md` for numbering contention, enum values, locks, chunked deletes and the
  failed-deploy recovery.

## Code Review
- Priority: correctness > security > performance > clarity > consistency.
- Approve when good enough; don't block on style that `cargo fmt` or ESLint already enforce.
- **After fixing a bug, search for sibling sites with the same pattern** (same query shape, same
  guard, same call) and fix them in the same pass or name them. #626 gated release writes on ARTCC
  scope; the four FCA writes beside them were the same hole (#636), and an unscoped edit could
  relabel an FCA into the caller's ARTCC and release on it.
- **After changing an enum variant, a public signature or a shared type, grep the whole workspace for
  the old shape.** Branches merged alongside yours still use it; only an `--all-targets` build of
  `next` sees them (#543, #583, #649).
- **A "mirror of X" fix copies X's assumptions.** Diff the two request and model types before
  accepting it. On #488 `update_advisory` copied `update_tmi`'s clearing of `decoded`, a no-op for
  TMIs, which derive it, and a silent loss for advisories, which take it from the caller.

## Advisory locks — alignment matters

Two paths that must exclude each other take the SAME lock key. Different keys are independent locks
and exclude nothing. The FAA surface seed and its re-pull share `SEED_LOCK_KEY` in
`repos/faa_surface_seed.rs` for exactly that reason.

Before adding a lock, find the existing key for the resource (`tmu.advisory:{facility}:{day}`,
`sector_consolidation:{artcc}`) and reuse it. Lock scope matches the racing surface: a lock around
the write that does not also cover the read that decided it fixes nothing. A transaction-scoped
`pg_advisory_xact_lock` releases however the transaction ends; prefer it to a session lock.

## A guard justified by one path goes inside that path

When a new rule exists because of a new path (a machine credential, a public route, a new caller of
the trajectory model), put it behind the condition that selects that path. Added to a shared helper
it applies to every other path too, and nothing fails, because the new tests only exercise the new
path. Before adding a check to a shared function, list its callers and the paths that reach it (a
national-scope and a facility-scope principal; a user, an API key and a service account; each
trajectory caller), and write one test per untouched path asserting it still accepts what it
accepted before. `Attribution::machine_actor` is the shape: the conditional-write rule applies to a
machine and is `None` for a person.

The tell in review: existing tests for the untouched paths gain a changed fixture or a new setup line
in the same branch. Those tests did not need it; the new rule reached them.

**A preview and the operation it previews run one code path, not two copies.** The
`backend/audits/*.sql` scripts apply the real migration inside a transaction and diff the effect
rather than re-implementing it. A copy that "mirrors" the real thing stays right only until one of
them changes.

## Moving or narrowing a value is not done until every reader moves

Grep the old column, field or function across `backend/`, `discord/`, `crates/`, `web/` and
`packages/`; the generated client in `packages/api-client` and the web's own mirrors of backend rules
(`web/src/lib/permissions.ts`) are separate edits. Writers move too, including migrations and jobs.

**Every figure of the same quantity has to agree once you narrow one.** Before #543, `/me`'s
`tmu_national` came from one permission resolver and its permission tree from another, so one
response could say a user held a permission nationally beside a tree that omitted it. One resolver
now answers both.

**A flag that is only ever set is latched**, so one earned under the old basis stands for the life
of the record. Tell: a pass whose every write is `= true`.

## Switching a path on a current setting must not strand what the old setting produced

Where a setting selects which format a record or credential gets, a record that already holds one
made under the old value keeps working; only new ones follow the new value.

**A credential or client already handed out is the old setting's artifact too.** Tokens in the wild
carry `ois_pat_`, `ois_sa_` and `ois_dsk_` prefixes, and the bearer router dispatches on them; #384
narrowed the router only after checking every service-account token had been minted with its prefix.
Bots and desktop apps from the previous release keep calling during a deploy: `ack_job` accepts an ack
without `attempt` so an older bot is not rejected (#472). Before changing a token format, a required
field or a route a deployed client calls, decide out loud: accept the breakage and say so in the PR,
accept both shapes for a bounded period, or reissue. The suite cannot see this; every test mints its
credential in the new format.

## A link's host comes from config, not from the request

A URL to OIS that is handed to a third party or stored for later (today the VATUSA webhook
receiver) is built from `OIS_PUBLIC_URL` (`feed::vatusa::webhook_url`), never from `Host` or
`X-Forwarded-Host`, which a caller controls. `handlers/auth.rs` reads those headers only to compare
the request's origin against the configured redirect URI, never to build a link.

## A sentinel written in a catch block retires the record for good

When a backlog is selected by an empty column (`where processed_at is null`), never fill that column
on failure. A placeholder written in the error arm removes the record from the candidate query
permanently, and the work never runs again. Tell: the error arm writes the column the candidate query
filters on.

Bound the retry with an attempt counter the query excludes on, not a time window.
`integration.outbound_jobs` is the pattern: `attempt_count`, a linear backoff on `next_attempt_at`,
and `failed` after `MAX_ATTEMPTS` (8), which stays queryable.

## Retry and backoff: bound the run, not just the call

A retry can make failure slower than having none, and tests do not wait out the sleep, so reason
about the callers and then pin it in a test.

- **Budget the run.** A caller that loops and retries multiplies the per-call worst case by the
  collection size. Every external call carries a timeout (the METAR client's is 15 s); a pass over
  many items stops waiting once its budget is spent and reports what it skipped.
- **Back off on the source's own signal.** The feed poller backs off to `STALE_POLL_BACKOFF_SECS`
  (30 s) after `MAX_CONSECUTIVE_STALE_POLLS` polls that returned no new publish (#92). Honor a
  `Retry-After` where the vendor sends one, discard a value that does not parse to a sane wait, and
  log which source set the wait.

## "Lazy fix" anti-pattern

Detecting a symptom and bailing gracefully (a cap that hides growth, "skip this flight", a sweep "to
catch any that fall through", a fallback write) is NOT a fix. It's a band-aid that hides the root
cause. Always:
1. Find what's actually causing the symptom.
2. Fix THAT.
3. The detect-and-bail can stay only as defense in depth, never as the primary fix.

#355's cruise-band cap in `ground_speed_at` was a workaround over wrong profile samples, so ETAs
through `time_between` stayed wrong; #361 fixed the samples and removed the cap.

## Trace before fix

Before proposing a change for a "data missing / job not run / value not written" symptom: read the
route → handler → every function it calls → the contended write. Enumerate EVERY path that could
write the same data (each handler, each `jobs.rs` pass, the feed collectors, the bot's ack path, any
migration that rewrites rows). Form a hypothesis only after the trace is on paper. Never propose a
sweep, retry or fallback before naming the primary path and explaining why it didn't fire.

## A config default is not the configured value

`rate_limit_per_min("RATE_LIMIT_USER_PER_MIN", 600)` tells you what happens when the variable is
unset. It does not tell you that it is unset. Reading that line and reasoning onward from the
fallback is the same mistake as reading a function signature and assuming the caller passes the
default.

**Print the resolved value.** Read `.env`, `.env.example` and the getter in `backend/src/config.rs`,
or log it from the running process.

**Then ask what else reads it.** A value in shared config has a blast radius past the change that
motivated it. Grep the key across `backend/`, `discord/`, `web/`, `.env*`, `docker-compose*`,
`deploy/` and `.github/`, and name what else moves. A green suite does not tell you what you broke
somewhere the suite does not look.

**A blank value is not an unset one, unless the getter says so.** `config::trimmed_env` treats
`X=` as unset, and so does `METRICS_TOKEN`. `DATABASE_URL` is read with `std::env::var` in
`state.rs`: blank is a connection string, and boot fails rather than running without a database.
Test absent and blank separately.

## Do not write the mechanism down until you have run the check that proves it

"Found it" has a high bar governs claiming root cause in chat, where being wrong costs a correction.
The bar is *higher* for anything durable: a commit body, a PR description, an issue comment, a code
comment. Those outlive the session, get read as settled fact by people who cannot cheaply re-derive
them, and are awkward to retract once pushed.

So the order is: verify, then write. A causal claim in a commit message is a claim, not commentary,
and a wrong one is worse than no explanation at all.

**A claimed limitation ends the investigation that would have disproved it.** On #537 two were false
and both excused skipping work: "the auto-publish job has no principal" (it carried the row's
`updated_by`) and "canceling fires the cancel job" (only the manual cancel handler enqueued it).
Before writing "X can't…", "X has no…" or "Y already does Z", find the line that makes it true and
cite it. For "Y fires Z", grep every place Z is enqueued.

**A precedence claim about a tool (env over file, which value wins) is checked with the tool's own
resolver, not its docs.** On #871 a plan said `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` replaces the config's
list; `cargo -Zunstable-options config get --show-origin` and `cargo build -v` showed cargo appends it.

If the reasoning is load-bearing and you have not run the check, say "evidence points at" in the
artifact too, or leave it out and state only what you did.

## "Found it" has a high bar

Before claiming root cause in a message, you must answer all three: (a) why does the symptom appear,
(b) why don't other paths writing the same data show it, (c) what does the fix change for adjacent
flows. If any answer is "I don't know yet", phrase as "evidence points at" or "strongest hypothesis
is", NOT "Found it" / "Root cause is X". Bugs here usually have a primary cause AND an interacting
condition.

## Comments — describe the code as it is now

- NO CIDs, callsigns, user ids, dates, "Reproduced YYYY-MM-DD", "Pre-fix", "Post-fix" or per-bug
  narratives in production comments. Repro evidence belongs in the PR description and the test.
- NO session context ("per review", "the operator flagged") or commit-message prose. An issue number
  beside the reason (`(#355)`) is the house pointer to the rationale; keep it a pointer, not a story.
- A calibration or domain comment names its reference (the FAA order, the vATCSCC document, the
  vatflow source it ports), never the flight that exposed the bug.
- Keep comments to a non-obvious technical reason for the code as it stands today. Anything longer
  belongs in a function name, a doc page or a rule file.
- **A `///` comment on a `ToSchema` model or field is contract**: utoipa emits it as the schema
  `description`, so editing it means regenerating the client (#591).
- **Correcting or retracting a claim greps for every copy** (code, docs, tests, PR and issue text)
  and fixes each in the same commit.
- **Name the rule, not the tally.** A comment counting its own call sites ("used by all five
  callers") asserts a completeness nothing checks, and is wrong when a sixth appears.
- **A branch that deletes a rule, guard or code path greps its own comments, tests and docs for
  references to it.**
