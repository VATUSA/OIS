#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/branch-naming-gate.sh.
#
#   bash bin/lib/branch-naming-gate.test.sh
#
# Builds a throwaway repo per branch name and feeds the gate a `git push`
# payload. Pure shell; macOS bash 3.2 and WSL2. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/branch-naming-gate.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/branch-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REPO="$WORK/repo"
git init -q -b next "$REPO"
git -C "$REPO" -c core.hooksPath=/dev/null commit -q --allow-empty -m init

# expect WANT BRANCH — WANT is allow (exit 0) or block (exit 2)
expect() {
    local want="$1" branch="$2" rc got
    git -C "$REPO" checkout -q -B "$branch"
    (cd "$REPO" && printf '{"tool_input":{"command":"git push -u origin HEAD"}}' | bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want $branch"; else bad "$branch (want $want, got $got)"; fi
}

expect allow feat/312/season-delta-check
expect allow fix/289/logbook-map-starburst
expect allow chore/a1b2c3/update-deps
expect block worktree-fix+289+logbook-map
expect block chore/update-deps
expect block internal/a1b2c3/update-deps
expect block feature/312/season-delta-check
expect block feat/312/season-delta-check-with-a-description-far-too-long
# The description is 2-4 hyphenated words.
expect allow chore/1617e87/aviodeck-workflow-sync
expect allow chore/a1b2c3d/session-learnings
expect allow chore/812/review-a1b2c3d
expect allow feat/47/save-event-replays-now
expect block feat/47/replays
expect block feat/47/save-the-event-replays-now
expect block feat/47/save--replays
expect block feat/47/save-replays-
expect block feat/47/-save-replays
# The limit is 50 characters inclusive: 9 for feat/312/ plus a two-word description of 41 or 42.
expect allow "feat/312/$(printf 'a%.0s' {1..20})-$(printf 'b%.0s' {1..20})"
expect block "feat/312/$(printf 'a%.0s' {1..21})-$(printf 'b%.0s' {1..20})"

# A payload the gate cannot parse must block; a non-push command passes untouched.
git -C "$REPO" checkout -q -B worktree-fix+289+logbook-map
(cd "$REPO" && printf 'not json' | bash "$GATE" >/dev/null 2>&1)
if [[ $? -eq 2 ]]; then ok "block an unparseable payload"; else bad "unparseable payload was not blocked"; fi
(cd "$REPO" && printf '{"tool_input":{"command":"git status"}}' | bash "$GATE" >/dev/null 2>&1)
if [[ $? -eq 0 ]]; then ok "allow a non-push command on worktree-fix+289+logbook-map"; else bad "non-push command was blocked"; fi

# run_on_bad_branch NAME WANT COMMAND — the command may hold quotes and newlines
run_on_bad_branch() {
    local rc got
    git -C "$REPO" checkout -q -B worktree-fix+289+logbook-map
    (cd "$REPO" && python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" | bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$2" ]]; then ok "$2 on worktree-fix+289+logbook-map: $1"; else bad "$1 (want $2, got $got)"; fi
}
run_on_bad_branch "git -C dir push"          block "git -C . push -u origin HEAD"
run_on_bad_branch "push chained after cd"    block "cd . && git push"
run_on_bad_branch "unparseable push line"    block "git push \"origin"
run_on_bad_branch "echoed push is a script"  block "echo \"git push origin HEAD\""
run_on_bad_branch "commit message naming a push" allow "git commit -m \"docs: git push origin HEAD\""
run_on_bad_branch "heredoc naming a push"    allow "$(printf 'cat > n.md <<%sEOF%s\ngit push\nEOF' "'" "'")"

# The session cwd is often the primary checkout, on next, which the gate skips: a push that runs in
# another checkout is judged by that checkout's branch, or blocked when the gate cannot follow it.
BADREPO="$WORK/bad"
GOODREPO="$WORK/good"
git clone -q "$REPO" "$BADREPO" && git -C "$BADREPO" checkout -q -b worktree-fix+289+logbook-map
git clone -q "$REPO" "$GOODREPO" && git -C "$GOODREPO" checkout -q -b fix/289/logbook-map-starburst
# run_from_next NAME WANT COMMAND
run_from_next() {
    local rc got
    git -C "$REPO" checkout -q -B next
    (cd "$REPO" && python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" | bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$2" ]]; then ok "$2 from next: $1"; else bad "$1 from next (want $2, got $got)"; fi
}
run_from_next "positive control: a push from next itself"   allow "git push origin fix/289/logbook-map-starburst"
run_from_next "a badly named destination from next"        block "git push origin x"
run_from_next "cd into a badly named checkout"              block "cd $BADREPO && git push -u origin HEAD"
run_from_next "cd into a well named checkout"               allow "cd $GOODREPO && git push -u origin fix/289/logbook-map-starburst"
run_from_next "git -C a badly named checkout"               block "git -C $BADREPO push -u origin worktree-fix+289+logbook-map"
run_from_next "GIT_DIR of a badly named checkout"           block "GIT_DIR=$BADREPO/.git git push"
run_from_next "cd joined by ; into a badly named checkout"  block "cd $BADREPO; git push"

# A refspec naming its destination creates that branch on origin, whatever the checkout is on, so the
# destination is held to the convention too (the cases OIS's earlier gate covered).
GOODWT="$WORK/goodwt"
git clone -q "$REPO" "$GOODWT" && git -C "$GOODWT" checkout -q -b feat/1/test-branch && git -C "$GOODWT" tag v1.0
# run_in_good NAME WANT COMMAND — from a well named checkout
run_in_good() {
    local rc got
    (cd "$GOODWT" && python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" | bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$2" ]]; then ok "$2 from feat/1/test-branch: $1"; else bad "$1 from feat/1/test-branch (want $2, got $got)"; fi
}
run_in_good "positive control: its own name"         allow "git push -u origin feat/1/test-branch"
run_in_good "bare push"                              allow "git push"
run_in_good "missing issue segment"                  block "git push origin HEAD:feat/save-event-replays"
run_in_good "unknown type"                           block "git push origin HEAD:feature/12/save-replays"
run_in_good "over 50 characters"                     block "git push origin HEAD:feat/12345/abcdefghijklmnopqrstu-vwxyzabcdefghijklmn"
run_in_good "refs/heads/ destination"                block "git push origin HEAD:refs/heads/Bad_Branch"
run_in_good "a second, badly named refspec"          block "git push origin feat/1/test-branch x:worktree-x"
run_in_good "a worktree-* destination"               block "git push origin HEAD:worktree-fix+289+logbook-map"
run_in_good "rework branch"                          allow "git push origin HEAD:chore/546/rework-d595743"
run_in_good "a one-word description"                 block "git push origin HEAD:feat/12/replays"
run_in_good "a five-word description"                block "git push origin HEAD:feat/12/save-the-event-replays-now"
run_in_good "the session-learnings branch"           allow "git push origin HEAD:chore/a1b2c3d/session-learnings"
run_in_good "branch held in a variable"              allow "git push -u origin \"\$BRANCH\""
run_in_good "delete a remote branch"                 allow "git push origin --delete Bad_Branch"
run_in_good "delete by :refspec"                     allow "git push origin :Bad_Branch"
run_in_good "push a tag"                             allow "git push origin v1.0"
run_in_good "push refs/tags/"                        allow "git push origin refs/tags/v2"
run_in_good "push --tags"                            allow "git push --tags origin"
run_in_good "next is master-protection's to judge"   allow "git push origin HEAD:next"

echo "The block names the worktree-* rename:"
out="$(git -C "$REPO" checkout -q -B worktree-fix+289+logbook-map && cd "$REPO" \
    && printf '{"tool_input":{"command":"git push -u origin HEAD"}}' | bash "$GATE" 2>&1)"
if [[ "$out" == *"rename it before the first push"* ]]; then ok "says to rename a worktree-* branch"; else bad "worktree-* message (${out:0:200})"; fi

echo "A refused line says why:"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "git config core.editor vim; git push origin x" \
    | (cd "$REPO" && bash "$GATE" 2>&1))"
rc=$?
if [[ $rc -eq 2 && "$out" == *"core.editor"* ]]; then ok "the parser's reason is in the message"; else bad "reason missing (rc=$rc: ${out:0:200})"; fi

echo ""
echo "branch-naming-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
