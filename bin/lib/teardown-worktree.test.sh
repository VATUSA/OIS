#!/bin/bash
# ==============================================================================
# Tests for bin/teardown-worktree.sh: what it refuses to remove, what it removes
# once every check passes, and when it keeps the local branch.
#
#   bash bin/lib/teardown-worktree.test.sh
#
# Each case builds a scratch repository with a bare "origin" and one linked
# worktree at .claude/worktrees/<NAME> (EnterWorktree's layout), and runs the script
# with stub psql and pg_isready first on PATH, so no real database is touched. The
# psql stub keeps the throwaway database's existence in a file and honours `drop
# database`. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/bin/teardown-worktree.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/teardown-worktree-test.XXXXXX")"
WORK="$(cd "$WORK" && pwd -P)"
SLEEPER=""
cleanup() { [[ -n "$SLEEPER" ]] && kill "$SLEEPER" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR WORKTREE_TEARDOWN_FROM_HOOK OIS_TEARDOWN_SKIP_DB
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

mkdir -p "$WORK/bin"
cat > "$WORK/bin/psql" <<'STUB'
#!/bin/bash
sql="${*: -1}"
echo "psql $sql" >> "$STUB_LOG"
case "$sql" in
    "select 1 from pg_database where datname = '$STUB_DB_NAME'") [[ -f "$STUB_DB" ]] && echo 1; exit 0 ;;
    "select 1 from pg_database"*) exit 0 ;;
    "drop database \"$STUB_DB_NAME\"") [[ -n "${STUB_DROP_FAILS:-}" ]] && exit 1; rm -f "$STUB_DB"; exit 0 ;;
esac
exit 1
STUB
printf '#!/bin/bash\nexit "${STUB_PG_RC:-0}"\n' > "$WORK/bin/pg_isready"
chmod +x "$WORK/bin/psql" "$WORK/bin/pg_isready"
export STUB_LOG="$WORK/stub.log"

# setup CASE: $MAIN (on next, pushed to a bare origin) with a worktree $WT at
# .claude/worktrees/$NAME on branch $BR = feat/1/<CASE>, one commit ahead. STUB_DB is
# the file standing for its throwaway database ois_wt_feat_1_<case>.
setup() {
    local base="$WORK/$1"
    rm -rf "$base"; mkdir -p "$base"
    git init -q --bare "$base/origin.git"
    MAIN="$base/main"
    git init -q -b next "$MAIN"
    printf '.env\n.claude/\n' > "$MAIN/.gitignore"
    git -C "$MAIN" add .gitignore && git -C "$MAIN" -c core.hooksPath=/dev/null commit -qm init
    git -C "$MAIN" remote add origin "$base/origin.git"
    git -C "$MAIN" push -q -u origin next
    printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\n' > "$MAIN/.env"
    BR="feat/1/$1"
    NAME="feat+1+$1"
    WT="$MAIN/.claude/worktrees/$NAME"
    git -C "$MAIN" worktree add -q -b "$BR" "$WT" next
    printf 'x\n' > "$WT/file.txt" && git -C "$WT" add file.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -qm work
    STUB_DB="$base/db.exists"
    STUB_DB_NAME="ois_wt_feat_1_$(printf '%s' "$1" | tr '-' '_')"
    export STUB_DB STUB_DB_NAME
    : > "$STUB_LOG"
}
push_branch() { git -C "$WT" push -q origin "$BR"; }

# teardown [ARGS...]: from $MAIN unless CWD is set; sets OUT (colors stripped) and RC.
teardown() {
    OUT="$(cd "${CWD:-$MAIN}" && PATH="$WORK/bin:$PATH" bash "$SCRIPT" "$@" 2>&1 | sed $'s/\033\\[[0-9;]*m//g'; exit "${PIPESTATUS[0]}")"
    RC=$?
}
has()      { case "$OUT" in *"$2"*) ok "$1" ;; *) bad "$1 (no '$2' in: $OUT)" ;; esac; }
lacks()    { case "$OUT" in *"$2"*) bad "$1 ('$2' in: $OUT)" ;; *) ok "$1" ;; esac; }
branch_exists() { git -C "$MAIN" show-ref --verify --quiet "refs/heads/$BR"; }
kept()     { if [[ -d "$WT" ]] && branch_exists; then ok "$1"; else bad "$1 (worktree or branch gone): $OUT"; fi; }
removed()  { if [[ ! -d "$WT" ]] && ! branch_exists; then ok "$1"; else bad "$1 (worktree or branch left): $OUT"; fi; }

echo "== removes a clean, pushed worktree =="
setup pushed; push_branch; touch "$STUB_DB"
teardown "$NAME"
[[ $RC -eq 0 ]] && ok "exit 0" || bad "exit $RC: $OUT"
removed "the worktree and its local branch are gone"
[[ ! -f "$STUB_DB" ]] && ok "the throwaway database is dropped" || bad "database left: $(cat "$STUB_LOG")"
grep -q "^psql drop database \"$STUB_DB_NAME\"" "$STUB_LOG" && ok "...by its derived name, quoted" || bad "drop call: $(cat "$STUB_LOG")"
has "...and says so" "Database $STUB_DB_NAME dropped"
git -C "$MAIN/../origin.git" show-ref --verify --quiet "refs/heads/$BR" && ok "the remote branch is untouched" || bad "remote branch deleted"
setup by-path; push_branch
teardown "$WT"
removed "a path selects the worktree"
has "...and a database never created is reported so" "never created"
setup by-branch; push_branch
teardown "$BR"
removed "a branch name selects the worktree"
setup merged
git -C "$WT" push -q origin "HEAD:refs/heads/next"
teardown "$NAME"
removed "a never-pushed branch whose commit is on origin/next is deleted"

echo "== keeps the branch when origin lacks its commits =="
setup unpushed
teardown "$NAME"
[[ $RC -eq 0 && ! -d "$WT" ]] && ok "the worktree goes (its commits stay on the branch)" || bad "unpushed: exit $RC, dir $([[ -d "$WT" ]] && echo left || echo gone)"
branch_exists && ok "...and the local branch is kept" || bad "unpushed branch deleted"
has "...and says why" "Local branch $BR kept: it has commits origin does not."
setup ahead; push_branch; printf 'w\n' > "$WT/more.txt" && git -C "$WT" add more.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -qm more
teardown "$NAME"
branch_exists && ok "a branch ahead of its pushed copy is kept" || bad "ahead branch deleted"

echo "== refuses =="
setup dirty; push_branch; printf 'y\n' > "$WT/untracked.txt"; touch "$STUB_DB"
teardown "$NAME"
[[ $RC -ne 0 ]] && has "an untracked file blocks" "BLOCKED: Worktree has uncommitted or untracked changes" || bad "dirty exit $RC"
kept "...and nothing is removed"
[[ -f "$STUB_DB" ]] && ok "...not even the database" || bad "database dropped before the refusal"
setup modified; push_branch; printf 'z\n' >> "$WT/file.txt"
teardown "$NAME"
kept "a modified tracked file blocks"
setup detached; git -C "$WT" checkout -q --detach
teardown "$NAME"
has "a detached HEAD no origin branch contains blocks" "which no origin branch contains"
[[ -d "$WT" ]] && ok "...and nothing is removed" || bad "detached worktree removed"
setup detached-pushed; push_branch; git -C "$WT" checkout -q --detach; git -C "$MAIN" fetch -q origin
teardown "$NAME"
[[ ! -d "$WT" ]] && ok "a detached HEAD on an origin branch is removed" || bad "detached-pushed: $OUT"
setup primary
git -C "$MAIN" checkout -q --detach
# Run from the worktree, so the inside-the-target refusal cannot answer for this one.
CWD="$WT" teardown "$MAIN"
has "the primary checkout is never removed, even detached from next" "is the primary checkout"
lacks "...and no later step runs" "Working directory is clean"
[[ $RC -ne 0 && -d "$MAIN/.git" ]] && ok "...and it exits non-zero with the checkout intact" || bad "primary exit $RC"
setup protected; git -C "$MAIN" checkout -q --detach && git -C "$WT" checkout -q next
teardown "$NAME"
has "a worktree on next is never removed" "main and next are never removed here"
[[ -d "$WT" ]] && ok "...and stays" || bad "worktree on next removed"
setup stranger; mkdir -p "$WORK/stranger/loose"
teardown "$WORK/stranger/loose"
has "a directory git does not list is not removed" "is not a registered worktree"
mkdir -p "$MAIN/.claude/worktrees/loose"
touch "$MAIN/.claude/worktrees/loose/keep.txt"
teardown loose
has "...nor one under .claude/worktrees" "is not a registered worktree"
[[ $RC -ne 0 && -f "$MAIN/.claude/worktrees/loose/keep.txt" ]] && ok "...which exits non-zero and leaves it in place" || bad "loose dir: exit $RC"
setup inside; push_branch
CWD="$WT" teardown "$NAME"
has "running from inside the worktree is refused" "never from inside the worktree it removes"
kept "...and nothing is removed"
CWD="$WT" WORKTREE_TEARDOWN_FROM_HOOK=1 teardown "$NAME"
removed "...unless the hook runs it there"

echo "== a worktree already gone =="
setup gone; push_branch; touch "$STUB_DB"
rm -rf "$WT"
teardown "$NAME"
[[ $RC -eq 0 ]] && ok "a name with no directory exits 0" || bad "gone exit $RC: $OUT"
[[ ! -f "$STUB_DB" ]] && ok "...after dropping the database derived from the name" || bad "gone: database left ($(cat "$STUB_LOG"))"
[[ -z "$(git -C "$MAIN" worktree list --porcelain | grep -F "$NAME")" ]] && ok "...and pruning the stale record" || bad "stale record left"
branch_exists && ok "...and keeps the branch (nothing proves its commit)" || bad "gone: branch deleted"

echo "== the throwaway database =="
setup unreachable; push_branch; touch "$STUB_DB"
STUB_PG_RC=2 teardown "$NAME"
has "an unreachable Postgres blocks" "cannot reach Postgres"
kept "...and nothing is removed"
OIS_TEARDOWN_SKIP_DB=1 STUB_PG_RC=2 teardown "$NAME"
removed "OIS_TEARDOWN_SKIP_DB=1 removes the worktree anyway"
has "...and says the database was left" "left behind"
[[ -f "$STUB_DB" ]] && ok "...which it was" || bad "database dropped while unreachable"
setup drop-fails; push_branch; touch "$STUB_DB"
STUB_DROP_FAILS=1 teardown "$NAME"
has "a drop that did not take blocks" "still exists after the drop"
kept "...and nothing is removed"

echo "== the worktree lock =="
setup stale-lock; push_branch
git -C "$MAIN" worktree lock --reason "claude session $BR (pid 999999 start 1)" "$WT"
teardown "$NAME"
removed "a lock whose process is gone is overridden"
setup live-lock; push_branch
sleep 30 & SLEEPER=$!
git -C "$MAIN" worktree lock --reason "claude session $BR (pid $SLEEPER start 1)" "$WT"
teardown "$NAME"
if [[ -r /proc/self/stat ]]; then
    removed "a live pid whose start time differs is a stale lock (Linux)"
else
    has "a live process holding the lock blocks" "in use by another live Claude session (pid $SLEEPER)"
    kept "...and nothing is removed"
fi
{ kill "$SLEEPER"; wait "$SLEEPER"; } 2>/dev/null; SLEEPER=""
setup self-lock; push_branch
git -C "$MAIN" worktree lock --reason "claude session $BR (pid $$ start 1)" "$WT"
if [[ -r /proc/self/stat ]]; then
    ok "(the caller's own lock is covered by its start time on Linux)"
else
    CWD="$WT" WORKTREE_TEARDOWN_FROM_HOOK=1 teardown "$NAME"
    removed "the caller's own lock is overridden from inside by the hook"
    setup self-lock-outside; push_branch
    git -C "$MAIN" worktree lock --reason "claude session $BR (pid $$ start 1)" "$WT"
    WORKTREE_TEARDOWN_FROM_HOOK=1 teardown "$NAME"
    has "...but not from outside, where a subagent may still be working" "this Claude session holds the worktree lock"
    kept "...and nothing is removed"
fi
setup bare-lock; push_branch
git -C "$MAIN" worktree lock --reason "held by hand" "$WT"
teardown "$NAME"
has "a lock naming no process blocks" "locked with no owning process"

echo
echo "teardown-worktree: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
