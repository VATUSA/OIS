# Code quality

Loads for every file. Review standards that apply in any language here. The project's
zero-tolerance rules and the "ask what would make it wrong" checklist are in `AGENTS.md`
§ Working rules; this file adds the review findings that keep recurring and does not restate them.
Stack specifics are in `rust-backend.md`, `web-frontend.md`, and `database-postgres.md`.

Sources: ported from the house code-quality rule; OIS lessons from #436, #488, #508, #537, #543,
#583, and #649.

## Unjustified scope

Ship what the issue asks for and nothing else. A module, endpoint, permission, config flag, job,
or migration that no requirement calls for is a **MAJOR** review finding and comes out of the diff
before merge, however small or useful it might one day be.

The test is whether you can name the requirement it serves. "It seemed useful", "we might need it
later", and "it was quicker to generalize" are not requirements. A `jobs.rs` pass nothing needs
is the standard case: it brings a schedule someone has to reason about, a failure mode nobody
monitors, and a test nobody wrote.

Two things are in scope by definition: a fix for a defect this branch introduced, and work the
user approved in the session. Anything else outside the issue's logical scope gets named in the
report, and `docs/github-issues.md` § Scope decides whether it becomes its own ticket.

This is the mirror of `AGENTS.md` zero-tolerance rule 2. That rule stops a half-wired feature; this
one stops a capability nobody asked for.

## Size and complexity are review guidance, not gates

Reviewers may flag these as **SUGGESTION**, never as a blocker:

- function longer than about 40 lines, or nesting deeper than 3 levels
- a module past about 500 lines that mixes unrelated concerns
- more than 5 parameters (clippy fails at 8; see `rust-backend.md`)
- a PR diff past about 600 lines that could split cleanly

A number over the line is a prompt to ask whether the code reads clearly, not a defect.

## Fail-open and silent success

An operation never reports success on a path that failed.

- **A swallowed error is a bug.** `let _ = fallible()`, `.ok()` that discards an `Err`, and
  `unwrap_or_default()` on an I/O result all hide failure. Handle it, propagate it, or log it
  **and** surface it in the return value.
- **A silent skip is the same bug.** `if let Some(x) = lookup { … }` with no `else` turns a
  misconfiguration into nothing happening. On #436 a renamed channel constant left every stored
  mapping resolving to `None`; the handler skipped the enqueue, returned 200, and logged nothing.
  Give the `else` a `tracing::warn!` at minimum.
- **Validation fails closed.** An unknown, unparseable, or unverifiable value is rejected, not
  passed through.
- **A background pass reports its failures.** A `jobs.rs` pass that catches, logs, and returns
  `Ok` is invisible to the job registry and to `/metrics`.
- **Sanctioned exception: an integration that is deliberately off.** With `DISCORD_BOT_TOKEN` or
  `VATUSA_API_KEY` unset (see `AGENTS.md` § Environment variables), the integration never runs, so
  there is nothing to surface. A configured integration whose call failed must surface it. Don't
  "fix" the unconfigured short-circuit into an error.

## No band-aid fixes

Detecting a symptom and bailing gracefully is not a fix: a retry loop, a sweep "to catch any that
fall through", a fallback write, a cap that hides growth. Find the cause and fix that. A guard may
stay as defense in depth, never as the primary fix. `AGENTS.md` zero-tolerance rule 4 is the
trace-before-fix rule; follow it before you propose anything.

When you trace, enumerate **every writer** of the data in question, not only the one that
reported: the HTTP handler, each `jobs.rs` pass, the feed refresh, the bot's ack path, and any
migration that rewrites rows.

## "Found it" has a high bar

Before claiming a root cause, answer all three: (a) why the symptom appears, (b) why the other
paths that write the same data don't show it, and (c) what the fix changes for the adjacent flows.
If any answer is "not sure yet", say "evidence points at" or "strongest hypothesis is". Bugs here
usually have a primary cause and an interacting condition.

## A config default is not the configured value

`AGENTS.md` zero-tolerance rule 1 states the rule. The practice:

- **Print the resolved value.** Read `.env`, `.env.example`, and the code that parses it
  (`backend/src/config.rs`), or log it from the running process. One command beats reasoning
  from the fallback.
- **Then ask what else reads it.** A value in shared config has a blast radius beyond the change
  that motivated it. Grep the key across `backend/`, `discord/`, `web/`, `.env*`, `docker-compose*`,
  and `.github/` before changing it, and name what else moves.

## Verify a claim before you write it down

A claimed limitation ends the investigation that would have disproved it. On #537 two such claims
were false and both excused skipping work: "the auto-publish job has no principal to check" (it
passed the row's `updated_by` as the actor) and "canceling fires the cancel job" (only the manual
cancel handler enqueued it). One of them was put to the owner as a constraint in a question, so
their decision was built on it. Every gate passed.

- Before writing "X can't…", "X has no…", or "Y already does Z" anywhere (a comment, a PR, a
  question to the owner), find the line of code that makes it true and cite it.
- For "Y fires Z", grep every place Z is enqueued or emitted.
- A claim that a test protects something is also a claim. Mutate what it protects and watch the
  test fail before you write the sentence; see `test-quality.md`.

## A "mirror of X" fix copies X's assumptions

When a fix is described as mirroring a sibling, diff the two **request and model types** before
accepting it. On #488 `update_advisory` copied `update_tmi`'s clearing of `decoded`. That is a
no-op for TMIs, which derive `decoded`, but advisories take a caller-supplied `decoded`, so the
copy silently discarded a value the caller sent. The copy is wrong exactly where the types differ.

## After a fix, grep for the siblings

The first review almost never finds the sibling sites.

- After fixing a bug, search the codebase for the same pattern: same query shape, same guard,
  same call. Fix them in the same pass or name them in the report.
- After changing an enum variant or a public signature, grep the whole tree for the old shape.
  On #543 and #583 each PR updated every call site it could see; siblings merged alongside them
  kept the old shapes and `next` stopped compiling at untouched sites. #649 replaced a raw
  `broadcast::Sender`, and three call sites still called `.send()`.

## Never remove a check to make it pass

Don't delete a failing test, weaken an assertion, add `#[allow(...)]` or `eslint-disable` without
a stated reason, or skip a hook with `--no-verify`. Fix the error. There are two exceptions: a
failure you have confirmed is pre-existing on `origin/next`, which you say so in the PR; and the QA
reviewer's `.claude/` rules-branch push, which `.claude/agents/ticket-reviewer.md` (Phase C) makes
with `--no-verify` because it skips only the pre-push clippy run and the branch carries no code.

## Review priority

Correctness, then security, then performance, then clarity, then consistency. Don't block on
style that `cargo fmt` or ESLint already enforces.
