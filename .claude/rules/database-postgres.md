---
paths:
  - "backend/migrations/**"
  - "backend/audits/**"
  - "backend/src/repos/**"
  - "backend/src/jobs.rs"
  - "backend/src/feed/stats/**"
  - ".github/scripts/check-migration-versions.sh"
---
# Database & Postgres Rules (not loaded at startup; read in full before writing a migration, a raw or aggregate query, a lock or a bulk iteration)

Companion to `code-quality.md` ("Database Migrations") and `AGENTS.md` § Conventions & gotchas
(numbered `NNNN_name.sql`, applied on startup, text UUID keys, `touch_updated_at`, check-constrained
statuses, FK cascades) and § Testing & verification (`#[sqlx::test]`). This rule captures what has gone
wrong anyway.

---

## Test database vs production
- Tests run on real Postgres 17: each `#[sqlx::test]` gets its own throwaway database with every
  migration applied, locally and in CI. There is no second engine to diverge from production.
- Repos use runtime queries, so a misspelled column, a `group by` Postgres rejects or a type mismatch
  compiles and fails only when the query runs. Every new or changed query needs a test that executes
  it.
- The dev database in `.env` `DATABASE_URL` is shared by every worktree — **NEVER** `just reset`,
  drop, truncate or bulk-delete against it.
- DDL is transactional in Postgres: `begin; … rollback;` undoes a `create`, `alter` or `drop` too,
  so a probe on a real database can be rolled back. `create index concurrently` is the exception; it
  cannot run in a transaction at all.

## A local production copy has a date on it — check it before quoting a number

A production snapshot (what `backend/audits/*.sql` run against, via `$SNAPSHOT_URL`) is a restore, not
a live mirror, and it is stale by however long ago it was taken. The table still answers and nothing
in the result says when the snapshot stopped.

Before a production figure reaches an issue comment, a PR description or a deploy step:

- Check the snapshot's age: `max(created_at)` from a table written continuously
  (`access.audit_logs`), and ask whether that recency is plausible.
- Quote the date beside the number. "13 of 23 holders" is unfalsifiable; "13 of 23 as of the
  2026-10-01 snapshot" tells the next reader what to re-check.
- Treat "nothing has arrived in N days" with particular suspicion. An absence is precisely what a
  stale copy looks like.
- Counts only, never a CID, name or email (`data-protection.md`).

## Probe a migration before commit

Apply the migration to a throwaway database inside a transaction and roll it back, against data
shaped like production: an empty test database proves the syntax, not the data change. For a
migration that changes access or other effective state, ship a read-only audit that applies the real
file in a transaction on a snapshot, diffs the effect and rolls back;
`backend/audits/0094_role_seed_effect.sql`, `0099_collapse_effect.sql` and `0130_retag_effect.sql`
are the pattern.

## Failed-deploy migration recovery
- sqlx runs each migration file in one transaction together with its `_sqlx_migrations` row. A
  migration that failed rolled back whole and left no row, so it is safe to fix the file and redeploy,
  as long as no database has applied it. Confirm first:
  `select version, success, checksum from _sqlx_migrations where version = NNNN;`.
- An applied migration's checksum is checked at every startup. Editing an applied file stops the
  backend booting on every database that ran it. Add a new migration instead.
- A database holding a version your branch lacks refuses to boot your backend ("previously applied
  but is missing"). That is another branch's migration on the shared dev database, not a broken
  branch: boot against a throwaway database ("Running against a real database").
- A file starting `-- no-transaction` runs outside a transaction and can half-apply. Give such a file
  its one statement (`create index concurrently`) and nothing else.

## Picking a migration number: scan branches, not PRs

Two files with one version both apply, and the second fails on `_sqlx_migrations`' primary key,
leaving the database half-migrated (#569). `check-migration-versions.sh`, the
`migration_versions_are_unique` test and the `migrations` CI job catch a duplicate inside one tree;
`.claude/hooks/migration-gate.sh` refuses a commit whose number another `origin` branch holds, as of
your last fetch. None of them sees a branch pushed after that.

Sessions push branches long before they open PRs: #584's `0103` was on a branch with no PR when #585
picked the same number. Scan `origin/next` and every remote branch, pick above the highest, and
**re-scan immediately before pushing** (on #656 the re-scan fired: `0114` had been taken while the
rework was gating). The scan exits non-zero on a clash, so chain the push on it:

```bash
mine=0132   # the number your branch adds
git fetch -q --prune
taken=$( { git ls-tree --name-only origin/next backend/migrations/
  for b in $(git branch -r | grep -v -e HEAD -e "origin/$(git branch --show-current)\$"); do
    git diff --name-only --diff-filter=A origin/next..."$b" -- backend/migrations 2>/dev/null
  done; } | grep -oE 'migrations/[0-9]+' | grep -oE '[0-9]+$' | sort -u )
echo "highest taken: $(echo "$taken" | tail -1)"
! echo "$taken" | grep -qx "$mine" && git push -u origin "$(git branch --show-current)"
```

A gap is harmless and renumbering upward is always safe; a duplicate is fatal.

## Enum values come from the newest migration

A `create table`'s `check (status in (…))` may have been replaced by a later `alter table … drop
constraint … add constraint`. On #61 a query filtered on `'claimed'`, a value
`0050_ace_event_claims.sql` had removed, so the clause was dead. Before writing SQL that names a status
or enum value, in review as well as when writing:

```bash
grep -rn "check (<column>" backend/migrations/
grep -rn -A3 "alter table <schema>.<table>" backend/migrations/
```

`alter table` and `add constraint` usually sit on separate lines, so the second needs `-A`. Read the
newest hit and use that value set. New taxonomies stay `text` with a named check constraint; OIS uses
no Postgres `enum` types.

## Indexes
- Index the columns used in `where`, `order by` and joins. Postgres does not index a foreign-key
  column for you; add the index in the same migration.
- Prove a new query's plan with `EXPLAIN (ANALYZE, BUFFERS)` against a throwaway copy with realistic
  row counts (the `perf-check` skill); an empty test database plans everything as a sequential scan.

## Decode the type Postgres returns, not the column's

Aggregates change types: `count(*)` is `bigint`, `sum(integer)` is `bigint`, `sum(bigint)` and
`avg(…)` are `numeric`. sqlx refuses to decode a mismatch at runtime, so the query compiles and the
endpoint 500s on every call. `repos::stats::storage_forecast` did exactly that until review caught
it; it now casts with `::bigint`. Cast in the SQL to the Rust type you decode into, and test the query
against rows that make the aggregate non-empty.

## `fetch_all` holds the whole result set; stream with `fetch`

`fetch_all` materializes every row before the first one is used. On a large table that is the whole
table in memory per replica: the taxi-sample reload OOM-killed the backend until it moved to
`.fetch(pool)` and `try_next()` (#776, `repos/stats.rs`). Stream anything that grows with network
traffic or time, and build compact structures as rows arrive.

Paginate a bulk pass by key (`where id > $last order by id limit $n`), never by `offset`, which is
O(n) at depth and skips or repeats rows under concurrent writes.

## `for update` locks nothing outside a transaction

`select … for update` run on the pool autocommits: the row lock is taken and released in the same
statement. Take it on `&mut *tx` and hold the transaction across the read and the write it protects.
`select max(…) … for update` is rejected outright, and a row lock cannot protect "the first row of the
day"; `repos::tmu::allocate_advisory_number` serializes with `pg_advisory_xact_lock` on a key instead,
with the unique constraint as the backstop. The same goes for a session advisory lock taken on a pooled
connection: it outlives the request (`code-quality.md` → "Fail-Open / Silent Success").

## Migration chunked-delete pattern

A sqlx migration is one transaction, so chunking inside it releases no locks and holds every deleted
row until commit, on a deploy where the old replicas are still serving. A delete or rewrite of many
rows belongs in a job pass, chunked, each chunk its own transaction, with a termination ceiling so a
concurrent writer cannot keep the loop alive:

```rust
for _ in 0..MAX_BATCHES {
    let n = sqlx::query("delete from t where id in (select id from t where <predicate> limit 5000)")
        .execute(pool).await?.rows_affected();
    if n == 0 { return Ok(()); }
}
Err("delete still matching rows after MAX_BATCHES; a concurrent writer?".into())
```

Keep a migration that takes an `ACCESS EXCLUSIVE` lock (`alter table`, a non-concurrent
`create index`) short: requests on the old replicas queue behind it for its whole duration.

A migration or job that deletes rows says in a comment that the data is unrecoverable.

## Migrations are forward-only
- sqlx has no down migrations. A mistake is fixed by a new migration, never by editing an applied one.
- **A data migration that rewrites a column cannot restore it.** Say in its header comment what it
  changed and what repairs it if wrong (a re-sync, the next write). Identify the rows it touches
  before it overwrites the evidence that identifies them; the audit script is where that list lives.

## Drop what a guard found, by the name it found

A check constraint declared inline gets a name Postgres chose. `drop constraint if exists <guess>`
silently no-ops when the guess is wrong, the old constraint stays in force, and the first insert of
the new value fails at runtime. Find it in `pg_constraint` and drop that name, then add the
replacement with an explicit name so no later migration has to guess
(`0095_advisory_window_and_package_item.sql`, `0050_ace_event_claims.sql`).

## Renaming a stored key needs a data migration

Several features match on a logical name stored in a row: Discord channel names in
`integration.discord_channels`, permission names, role names. Renaming the constant without moving the
rows breaks every deployed install, usually silently. On #436 a renamed channel constant left every
stored mapping resolving to `None`; the publish skipped the post, returned 200, and logged nothing.

- Ship a migration that moves the existing rows. A deploy note asking an admin to remap fails until
  someone acts, and the failure is invisible.
- Copy `0041_stats_perm_rename.sql`: insert the new row, repoint every referencing table, then delete
  the old row, so existing and fresh databases converge.
- Guard against the new name already existing, since these tables carry unique constraints and one
  collision takes the whole deploy's migrations down (`0083` skips a guild holding both names).
- Test it with `#[sqlx::test]`: seed the old name, run the file through
  `include_str!("../../migrations/NNNN_….sql")`, assert the lookup resolves, and pin the case where
  both names already exist (`the_ntml_channel_mapping_survives_the_rename` in `handlers/tmu.rs`).

## Destructive statements

A migration or job that deletes or updates rows runs automatically on deploy, so a surviving mutant
here means mass data loss, not a wrong number. Mutate every predicate of the `where`, one at a time,
against a fixture that starts from the most common real row and seeds one surviving neighbor per
predicate (`test-writing.md` → "Mutation traps"; #550, #656, #706). A repo `delete` keyed on two
columns tested against one row proves nothing: on #706 dropping either predicate stayed green.

## A data migration bypasses every handler side effect, and the caches they reload

A migration writes rows. It does not do what the handler for the same write does: force-reload the
`AppState` cache, publish a realtime topic, write an audit row, enqueue an outbound job.

```
handler write ─▶ rows ─▶ force-reload (this replica) ─▶ publish topic ─▶ audit row
migration     ─▶ rows ─╳ no reload  ─╳ no topic  ─╳ no audit  ─╳ no Discord post
                         │
                         ▼
   new replica: migrations run before the refresh jobs spawn, so its caches load the new rows
   old replicas (rolling deploy): keep the old cache until their next refresh tick
   clients: refetch at the next topic or the 60 s fallback poll
```

**A data migration that changes what a cached table holds is safe only if the next refresh tick
picks it up**; a cache with no refresh pass (loaded once at startup) never sees it on a replica that
was already running. An access-changing migration's record is its audit script, since no audit row
is written. A change Discord should show needs its job enqueued by code, not by the migration.

Test it: run the migration with `include_str!` in a `#[sqlx::test]`, then call the cache's loader
(`repos::aircraft_profiles` for the profile table) and assert what the feed would read.

## Running against a real database

The shared dev database carries other branches' migrations, so booting your branch's backend against
it can fail with "migration N was previously applied but is missing". Each linked worktree gets its
own throwaway database, `ois_wt_<id>` (`bin/lib/portable.sh` → `worktree_db_name`;
`bin/setup-worktree.sh` prints it and `bin/teardown-worktree.sh` drops it). Point the backend at it on
a spare port:

```bash
psql postgres://ois:ois@127.0.0.1:5432/postgres -c 'create database ois_wt_<id>;'
DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois_wt_<id> BIND_ADDR=127.0.0.1:3407 \
  cargo run -p ois-backend
```

Migrations apply on startup. Don't set `DATABASE_URL` to an empty string: only an unset variable runs
the backend without a database; a blank one fails the boot.

Stop what you started by the PID you recorded when you started it (`$!`, or a pidfile in your
scratchpad). Never `pkill -f target/debug/ois-backend` or another path pattern: it matches every
worktree's backend and kills other sessions' stacks.
