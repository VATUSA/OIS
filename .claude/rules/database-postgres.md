---
paths:
  - "backend/migrations/**"
  - "backend/src/repos/**"
---

# Postgres: migrations and repos

Loads when you read or edit a migration or a repo module. The migration conventions (numbered
`NNNN_name.sql`, applied on startup, never edit an applied one, text UUID keys, `touch_updated_at`,
check-constrained statuses, FK cascades), the migration-number contention and its three guards,
and the `#[sqlx::test]` setup are in `AGENTS.md` § Conventions & gotchas and § Testing &
verification. Read those first; this file adds what has gone wrong anyway.

Sources: OIS lessons from #61, #436, #550, #569, #584, #585, #656, #706, and #795.

## Picking a migration number: scan branches, not PRs

`AGENTS.md` gives the open-PR scan. It is not enough: sessions push branches long before they
open PRs, and #584's `0103` was on a branch with no PR when #585 picked the same number. Scan
`origin/next` and every other remote branch, pick above the highest, and **re-scan immediately
before pushing** (on #656 the re-scan fired: `0114` had been taken while the rework was gating).
The scan below exits non-zero on a clash, so chain the push on it:

```bash
mine=0127   # the number your branch adds
git fetch -q --prune
taken=$( { git ls-tree --name-only origin/next backend/migrations/
  for b in $(git branch -r | grep -v -e HEAD -e "origin/$(git branch --show-current)\$"); do
    git diff --name-only --diff-filter=A origin/next..."$b" -- backend/migrations 2>/dev/null
  done; } | grep -oE 'migrations/[0-9]+' | grep -oE '[0-9]+$' | sort -u )
echo "highest taken: $(echo "$taken" | tail -1)"
! echo "$taken" | grep -qx "$mine" && git push -u origin "$(git branch --show-current)"
```


A gap is harmless and renumbering upward is always safe; a duplicate half-migrates the database at
startup (`.github/scripts/check-migration-versions.sh` explains why).

## Enum values come from the newest migration

A `create table` migration's `check (status in (…))` may have been replaced by a later
`alter table … drop constraint … add constraint`. On #61 a query filtered on `'claimed'`, a value
`0050_ace_event_claims.sql` had removed, so the clause was dead. Before writing SQL that names a
status or enum value:

```bash
grep -rn "check (<column>" backend/migrations/
grep -rn -A3 "alter table <schema>.<table>" backend/migrations/
```

The first grep finds every value set; `alter table` and `add constraint` usually sit on separate
lines, so the second needs `-A`. Read the newest hit and use that value set. Do this in review too, not only when writing.

## Renaming a stored key needs a data migration

Several features match on a logical name stored in a row: Discord channel names in
`integration.discord_channels`, permission names, role names. Renaming the constant without moving
the rows breaks every deployed install, usually silently. On #436 a renamed channel constant left
every stored mapping resolving to `None`; the handler skipped the post, returned 200, and logged
nothing.

- Ship a migration that updates the existing rows. A deploy note asking an admin to remap is not
  equivalent: it fails until someone acts, and the failure is invisible.
- Copy `backend/migrations/0041_stats_perm_rename.sql`: insert the new row, repoint every
  referencing table, then delete the old row, so existing and fresh databases converge.
- Guard against the new name already existing, since these tables carry unique constraints and a
  bare `update` collision takes the whole deploy's migrations down:
  `… and not exists (select 1 from t other where other.<scope> = c.<scope> and other.name = '<new>')`.
- Test it with `#[sqlx::test]`: seed the old name, run the migration through
  `include_str!("../../migrations/NNNN_….sql")` (precedent: `backend/src/repos/access.rs:1282`),
  and assert the lookup resolves. Pin the case where both names already exist.

## Destructive statements

A migration or job that deletes or updates rows runs automatically on deploy, so a surviving
mutant here means mass data loss, not a wrong number.

- Mutate every predicate of the `WHERE`, one at a time, against a fixture that starts from the
  most common real row and seeds one surviving neighbor per predicate. The procedure is in
  `test-quality.md` § What mutation proves (#550, #656, #706).
- For a data migration that changes access or other effective state, ship a read-only audit
  script that applies the migration in a transaction on a production snapshot, diffs the effective
  state, and rolls back. `backend/audits/0094_role_seed_effect.sql` and
  `backend/audits/0099_collapse_effect.sql` are the pattern.

## Repo queries

- All SQL lives in `backend/src/repos/` (`AGENTS.md` § Backend shape). Bind every value; a
  `format!` is only for trusted code constants, with a comment saying so (`secure-coding.md`).
- Repos use runtime queries, so a typo in a column name compiles and fails only when the query
  runs. Every new or changed query needs a `#[sqlx::test]` that executes it.
- A repo `delete` or `update` keyed on several columns gets a neighbor row per key in its test.
  On #706 a two-predicate `delete … where artcc = $1 and sector_id = $2` was tested against one
  row, and dropping either predicate stayed green.
- A query that scans every member (a bulk pass, a candidate list, a division-wide diff) gets an
  `EXPLAIN ANALYZE` against a throwaway database seeded at division scale (10,000+ users) before it
  ships. A unit fixture of a few rows can't show the plan. Watch for a correlated `EXISTS` or
  `IN` over a CTE: a CTE referenced twice is materialized without an index, so each outer row
  rescans it. On #795 the reset's candidate query ran 2 × 14,850 such scans, 31 s at 15,000
  users and quadratic beyond. A set-based diff (`FULL JOIN … USING (…)`, keep the rows with one
  side null) returned the same rows in 185 ms. Put the timing in the PR.
- Feed-visible data is loaded into an `AppState` cache by a job, never queried from `feed/`
  (`AGENTS.md` § The live feed subsystem).

## Running against a real database

The shared dev database carries other branches' migrations, so booting your branch's backend
against it can fail with "migration N was previously applied but is missing". Create a throwaway
database and point the backend at it on a spare port:

```bash
docker exec ois-postgres-1 psql -U ois -d postgres -c 'create database ois_tmp;'
DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois_tmp BIND_ADDR=127.0.0.1:3407 \
  ./target/debug/ois-backend
```

Drop it afterwards. Don't set `DATABASE_URL` to an empty string; only an unset variable disables
the database.
