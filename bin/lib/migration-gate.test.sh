#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/migration-gate.sh.
#
#   bash bin/lib/migration-gate.test.sh
#
# Builds a primary checkout on next with two migrations, a fake origin branch that
# already took migration 3, and a linked worktree, then feeds the gate `git commit`
# payloads. A staged (or same-line `git add`) migration whose version another origin
# branch holds must block (#569); a free number, your own branch's copy and a commit
# without migrations pass. Pure shell; macOS bash 3.2. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/migration-gate.sh"
WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/migration-gate-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

PRIMARY="$WORK/primary"
WT="$WORK/wt"
{
    git init -q -b next "$PRIMARY"
    mkdir -p "$PRIMARY/backend/migrations"
    echo 'select 1;' > "$PRIMARY/backend/migrations/0001_init.sql"
    echo 'select 2;' > "$PRIMARY/backend/migrations/0002_users.sql"
    git -C "$PRIMARY" add backend
    git -C "$PRIMARY" -c core.hooksPath=/dev/null commit -qm init
    git -C "$PRIMARY" update-ref refs/remotes/origin/next HEAD
    # Another open branch that already took migration 3.
    git -C "$PRIMARY" checkout -q -b feat/9/other-thing
    echo 'select 3;' > "$PRIMARY/backend/migrations/0003_other.sql"
    git -C "$PRIMARY" add backend/migrations/0003_other.sql
    git -C "$PRIMARY" -c core.hooksPath=/dev/null commit -qm other
    git -C "$PRIMARY" update-ref refs/remotes/origin/feat/9/other-thing HEAD
    git -C "$PRIMARY" checkout -q next
    git -C "$PRIMARY" branch -q -D feat/9/other-thing
    git -C "$PRIMARY" worktree add -q "$WT" -b feat/1/test-branch next
} >/dev/null 2>&1 || { echo "fixture setup failed" >&2; exit 1; }

# gate WANT NAME COMMAND [CWD] — WANT is allow (exit 0) or block (exit 2); the payload's cwd is CWD (default $WT)
gate() {
    local want="$1" rc got cwd="${4:-$WT}"
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}, "cwd": sys.argv[2]}))' \
        "$3" "$cwd" | (cd "$WORK" && bash "$GATE" >"$WORK/out" 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want: $2"; else bad "$2 (want $want, got $got: $(head -c 300 "$WORK/out"))"; fi
}

echo "A migration version another origin branch holds blocks:"
echo 'select 33;' > "$WT/backend/migrations/0003_mine.sql"
gate block "migration added in the same command collides" 'git add backend/migrations/0003_mine.sql && git commit -m "feat: x"'
gate block "...through cd ABSDIR" "cd $WT && git add backend/migrations/0003_mine.sql && git commit -m \"feat: x\"" "$WORK"
gate block "...through git -C" "git -C $WT add backend/migrations/0003_mine.sql && git -C $WT commit -m x" "$WORK"
gate block "...inside bash -c" "bash -c 'git add backend/migrations/0003_mine.sql && git commit -m x'"
git -C "$WT" add backend/migrations/0003_mine.sql
gate block "staged migration collides with an origin branch" 'git commit -m "feat: x"'
gate block "...and the block names both files" 'git commit -m "feat: x"'
if grep -q '0003_mine.sql collides with 0003_other.sql on origin/feat/9/other-thing' "$WORK/out"; then ok "the message names the collision"; else bad "message ($(cat "$WORK/out"))"; fi
gate allow "a heredoc written to a file naming a commit" "$(printf 'cat > notes.md <<%sEOF%s\ngit commit -m x\nEOF' "'" "'")"
git -C "$WT" rm -q --cached backend/migrations/0003_mine.sql
mv "$WT/backend/migrations/0003_mine.sql" "$WT/backend/migrations/0004_mine.sql"
git -C "$WT" add backend/migrations/0004_mine.sql
gate allow "staged migration with a free number" 'git commit -m "feat: x"'
gate block "zero-padding does not hide a collision" 'git add backend/migrations/3_mine.sql && git commit -m x'
git -C "$WT" update-ref refs/remotes/origin/feat/1/test-branch "$(git -C "$WT" commit-tree -p HEAD -m own "$(git -C "$WT" write-tree)")"
gate allow "same file on your own remote branch" 'git commit -m "feat: x"'
git -C "$WT" rm -q --cached backend/migrations/0004_mine.sql
gate allow "commit without migrations" 'git commit -m "feat: x"'
gate allow "unrelated command" 'git status'

echo "A git add of the directory or a glob stages every untracked migration in it:"
echo 'select 33;' > "$WT/backend/migrations/0003_mine.sql"
for add in "backend/migrations/" "backend/migrations" "./backend/migrations/" "backend" "backend/" \
    "backend/migrations/*.sql" "'backend/migrations/*.sql'" "backend/migrations/000?_*.sql" "backend/*" "'*.sql'" \
    "backend/migrations/." "." "':(glob)**/*.sql'" "web backend/migrations/0003_mine.sql"; do
    gate block "git add $add && git commit" "git add $add && git commit -m x"
done
gate block "...through git -C" "git -C $WT add backend/migrations/ && git -C $WT commit -m x" "$WORK"
gate block "...from a subdirectory" "cd $WT/backend && git add migrations && git commit -m x" "$WORK"
gate block "...a glob from a subdirectory" "cd $WT/backend && git add '*.sql' && git commit -m x" "$WORK"
gate block "...a parent from a subdirectory" "cd $WT/backend/migrations && git add .. && git commit -m x" "$WORK"
gate allow "positive control: a git add of other paths" "git add backend/src/a.rs && git commit -m x"
gate allow "...and of web/" "git add web && git commit -m x"
mv "$WT/backend/migrations/0003_mine.sql" "$WT/backend/migrations/0005_mine.sql"
gate allow "an untracked migration with a free number" "git add backend/migrations/ && git commit -m x"
printf '0005_mine.sql\n' > "$WT/backend/migrations/.gitignore"
mv "$WT/backend/migrations/0005_mine.sql" "$WT/backend/migrations/0003_ignored.sql"
printf '0003_ignored.sql\n' > "$WT/backend/migrations/.gitignore"
gate allow "an ignored file is not a candidate" "git add backend/migrations/ && git commit -m x"
rm -f "$WT/backend/migrations/0003_ignored.sql" "$WT/backend/migrations/.gitignore"

echo "A commit the gate cannot place, or a line it cannot read, blocks:"
gate block "cd to a directory the shell builds" 'cd "$DIR" && git commit -m x'
gate block "unparseable line" 'git commit -m "unterminated'
for payload in "" "this is not json"; do
    printf '%s' "$payload" | bash "$GATE" >/dev/null 2>&1
    if [[ $? -eq 2 ]]; then ok "block: payload '${payload:-<empty>}'"; else bad "payload '${payload:-<empty>}' was not blocked"; fi
done

echo ""
echo "migration-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
