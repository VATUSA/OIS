#!/bin/bash
# ==============================================================================
# Tests for bin/verify-worktree.sh: what it says about .env, Postgres, the shared
# dev database's migrations, node_modules against the lockfile, the worktree's own
# target dir, the branch name, the branch's merges and the installed git hooks.
#
#   bash bin/lib/verify-worktree.test.sh
#
# Runs the script in a linked worktree of a scratch repository with stub psql and
# pg_isready first on PATH, so no database is touched. Checks only the lines under
# test. Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/bin/verify-worktree.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/verify-worktree-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR RUSTC_WRAPPER
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

mkdir -p "$WORK/bin"
# psql prints the versions in $STUB_APPLIED (one per line) or fails with $STUB_PSQL_RC;
# pg_isready exits $STUB_PG_RC.
printf '#!/bin/bash\necho "psql $*" >> "$WORK_LOG"\n[ -n "${STUB_PSQL_RC:-}" ] && exit "$STUB_PSQL_RC"\nprintf "%%b" "${STUB_APPLIED:-}"\n' > "$WORK/bin/psql"
printf '#!/bin/bash\necho "pg_isready $*" >> "$WORK_LOG"\nexit "${STUB_PG_RC:-0}"\n' > "$WORK/bin/pg_isready"
chmod +x "$WORK/bin/"*
export WORK_LOG="$WORK/stub.log"

MAIN="$WORK/main"
g() { git -C "$MAIN" -c core.hooksPath=/dev/null "$@"; }
mkdir -p "$MAIN/backend/migrations"
git init -q -b next "$MAIN"
printf 'node_modules/\n.env\n' > "$MAIN/.gitignore"
printf 'lockfileVersion: 9.0\n' > "$MAIN/pnpm-lock.yaml"
printf 'select 1;\n' | tee "$MAIN/backend/migrations/0001_init.sql" > "$MAIN/backend/migrations/0002_more.sql"
g add -A && g commit -qm init
g update-ref refs/remotes/origin/next HEAD
g config core.hooksPath .githooks
WT="$WORK/wt/feat/1/thing"
g worktree add -q -b feat/1/the-thing "$WT"

# healthy: .env, a pnpm-installed node_modules matching the lockfile.
healthy() {
    printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\n' > "$WT/.env"
    rm -rf "$WT/node_modules"; mkdir -p "$WT/node_modules/.pnpm"
    cp "$WT/pnpm-lock.yaml" "$WT/node_modules/.pnpm/lock.yaml"
}
# verify: run the script in the worktree; sets OUT (colors stripped) and RC.
verify() {
    local esc
    : > "$WORK_LOG"
    esc="$(printf '\033')"
    OUT="$(cd "$WT" && PATH="$WORK/bin:$PATH" bash "$SCRIPT" 2>&1 | sed "s/${esc}\[[0-9;]*m//g"; exit "${PIPESTATUS[0]}")"
    RC=$?
}
# has NAME TEXT / lacks NAME TEXT: against $OUT
has()   { case "$OUT" in *"$2"*) ok "$1" ;; *) bad "$1 (no '$2' in: $OUT)" ;; esac; }
lacks() { case "$OUT" in *"$2"*) bad "$1 ('$2' in: $OUT)" ;; *) ok "$1" ;; esac; }

echo "== a healthy worktree =="
healthy
export STUB_APPLIED='1\n2\n'
verify
has "a linked worktree" "PASS Linked worktree"
has ".env with a local DATABASE_URL" "PASS DATABASE_URL points at the local Postgres (127.0.0.1:5432/ois)"
has "Postgres reachable" "PASS Postgres reachable at 127.0.0.1:5432"
grep -q '^pg_isready -q -h 127.0.0.1 -p 5432' "$WORK_LOG" && ok "...probed at the URL's host and port" || bad "pg_isready call: $(cat "$WORK_LOG")"
has "migrations match" "PASS the shared dev database's migrations match this branch's"
grep -q "^psql postgres://ois:ois@127.0.0.1:5432/ois -Atqc select version from _sqlx_migrations" "$WORK_LOG" && ok "...read from _sqlx_migrations at DATABASE_URL" || bad "psql call: $(cat "$WORK_LOG")"
has "node_modules matches the lockfile" "PASS node_modules matches pnpm-lock.yaml"
has "cargo builds into the worktree's own target dir" "PASS cargo builds into this checkout's own target/"
has "the git hooks are installed" "PASS git hooks installed (core.hooksPath=.githooks)"
has "the throwaway database is named after the worktree" "throwaway database name for booting the backend: ois_wt_thing"
has "a conventional branch name" "PASS Branch name: feat/1/the-thing"
has "forks from origin/next" "PASS Forks from origin/next (0 commits behind its tip)"
has "no foreign merges" "PASS No merge commits from other branches"
[ "$RC" -eq 0 ] && ok "...and the health check exits 0" || bad "healthy exit $RC: $OUT"

echo "== .env and Postgres =="
rm "$WT/.env"; verify
has "no .env fails" "FAIL .env missing — run: bash bin/setup-worktree.sh"
[ "$RC" -ne 0 ] && ok "...and the health check exits non-zero" || bad "exit $RC"
healthy; printf 'API_PORT=3000\n' > "$WT/.env"; verify
has "a .env without DATABASE_URL fails" "FAIL DATABASE_URL not set in .env"
healthy; printf 'DATABASE_URL="postgres://ois:ois@db.example.com:5433/ois" # remote\n' > "$WT/.env"; verify
has "a remote DATABASE_URL (quoted, with a comment) is a warning naming the host" "WARN DATABASE_URL points at db.example.com, not this machine"
grep -q '^pg_isready -q -h db.example.com -p 5433' "$WORK_LOG" && ok "...and that host and port are probed" || bad "pg_isready call: $(cat "$WORK_LOG")"
healthy; STUB_PG_RC=2 verify
has "an unreachable Postgres fails" "FAIL Postgres not reachable at 127.0.0.1:5432 — run: just up"
lacks "...and no migration check is attempted" "migrations match"

echo "== the shared dev database's migrations =="
healthy
STUB_APPLIED='1\n2\n3\n' verify
has "a migration the branch lacks is a warning naming it" "WARN the shared dev database has migration(s) 3 this branch lacks"
has "...with the boot failure it causes" "previously applied but is missing"
has "...and the throwaway database to use" "Boot against a throwaway database: ois_wt_thing"
lacks "...and does not pass" "migrations match"
STUB_APPLIED='1\n' verify
has "a migration the branch adds is a warning naming it" "WARN this branch adds migration(s) 2 the shared dev database lacks"
STUB_PSQL_RC=1 verify
has "an unreadable _sqlx_migrations is a warning, not a pass" "WARN could not read _sqlx_migrations"
printf 'select 1;\n' > "$WT/backend/migrations/0010_padded.sql"
STUB_APPLIED='1\n2\n10\n' verify
has "zero-padded file versions compare as numbers" "PASS the shared dev database's migrations match this branch's"
rm "$WT/backend/migrations/0010_padded.sql"

echo "== node_modules against the lockfile =="
healthy; rm -rf "$WT/node_modules"; verify
has "no node_modules fails" "FAIL node_modules missing"
healthy; printf 'lockfileVersion: 9.0\n# older\n' > "$WT/node_modules/.pnpm/lock.yaml"; verify
has "node_modules installed from another lockfile fails" "FAIL node_modules was installed from a different pnpm-lock.yaml"
lacks "...and does not pass" "PASS node_modules matches"
healthy; rm "$WT/node_modules/.pnpm/lock.yaml"; verify
has "node_modules pnpm did not install fails" "FAIL node_modules was not installed by pnpm"
healthy; rm -rf "$WT/node_modules"; ln -s "$MAIN" "$WT/node_modules"; verify
has "a symlinked node_modules fails" "FAIL node_modules is a symlink"
rm "$WT/node_modules"; healthy

echo "== the worktree's own target dir =="
CARGO_TARGET_DIR="$(cd "$MAIN" && pwd -P)/target" verify
has "a CARGO_TARGET_DIR shared with the main checkout fails" "FAIL CARGO_TARGET_DIR=$(cd "$MAIN" && pwd -P)/target is exported"
[ "$RC" -ne 0 ] && ok "...and the health check exits non-zero" || bad "shared CARGO_TARGET_DIR exit $RC"
CARGO_TARGET_DIR="$(cd "$WT" && pwd -P)/target/" verify
has "a CARGO_TARGET_DIR naming the worktree's own target/ passes" "PASS cargo builds into this checkout's own target/"
CARGO_BUILD_TARGET_DIR="$WORK/shared-target" verify
has "a CARGO_BUILD_TARGET_DIR elsewhere fails too" "FAIL CARGO_BUILD_TARGET_DIR=$WORK/shared-target is exported"
lacks "...and does not pass the target check" "PASS cargo builds into this checkout's own target/"
[ "$RC" -ne 0 ] && ok "...and the health check exits non-zero" || bad "CARGO_BUILD_TARGET_DIR exit $RC"
CARGO_BUILD_TARGET_DIR="$(cd "$WT" && pwd -P)/target" verify
has "a CARGO_BUILD_TARGET_DIR naming the worktree's own target/ passes" "PASS cargo builds into this checkout's own target/"
verify
lacks "no CARGO_TARGET_DIR is no warning" "CARGO_TARGET_DIR"

echo "== the git hooks =="
g config --unset core.hooksPath; verify
has "no core.hooksPath fails and names just setup" "FAIL git hooks not installed (core.hooksPath is '', not .githooks): run just setup once per clone"
[ "$RC" -ne 0 ] && ok "...and the health check exits non-zero" || bad "no hooks exit $RC"
g config core.hooksPath /dev/null; verify
has "another core.hooksPath fails" "FAIL git hooks not installed (core.hooksPath is '/dev/null'"
g config core.hooksPath .githooks; verify
has "core.hooksPath=.githooks passes" "PASS git hooks installed"

echo "== branch name and ancestry =="
git -C "$WT" branch -q -m worktree-feat+1+thing; verify
has "a worktree-* branch is a warning before /start renames it" "WARN Branch 'worktree-feat+1+thing' is the EnterWorktree name"
git -C "$WT" branch -q -m bad_name; verify
has "another bad name fails" "FAIL Branch 'bad_name' does not match {type}/{issue-or-hash}/{description}"
git -C "$WT" branch -q -m chore/1617e87/a-name-long-enough-to-run-past-fiftyx; verify
has "a description over 4 words fails" "FAIL Branch 'chore/1617e87/a-name-long-enough-to-run-past-fiftyx' does not match"
git -C "$WT" branch -q -m feat/1/thing2; verify
has "a one-word description fails" "FAIL Branch 'feat/1/thing2' does not match"
git -C "$WT" branch -q -m chore/1617e87/aviodeck-workflow-sync; verify
has "a 3-word description passes" "PASS Branch name: chore/1617e87/aviodeck-workflow-sync"
git -C "$WT" branch -q -m chore/1617e87/abcdefghijklmnopqrstuvw-abcdefghijklm; verify
has "a 2-word, 51-character name fails on length" "FAIL Branch 'chore/1617e87/abcdefghijklmnopqrstuvw-abcdefghijklm' is 51 chars (max 50)"
git -C "$WT" branch -q -m feat/1/the-thing
g checkout -q -b fix/2/other && printf 'x\n' > "$MAIN/other.txt" && g add other.txt && g commit -qm other && g checkout -q next
git -C "$WT" -c core.hooksPath=/dev/null merge -q --no-ff --no-edit fix/2/other
verify
has "a merge of another feature branch fails" "FAIL Branch carries merge commits from branches other than next and main"
git -C "$WT" reset -q --hard origin/next
g checkout -q -b up next && printf 'y\n' > "$MAIN/up.txt" && g add up.txt && g commit -qm up && g update-ref refs/remotes/origin/next HEAD && g checkout -q next
git -C "$WT" -c core.hooksPath=/dev/null merge -q --no-ff --no-edit origin/next
verify
has "a merge of origin/next passes" "PASS No merge commits from other branches"

echo
echo "verify-worktree: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
