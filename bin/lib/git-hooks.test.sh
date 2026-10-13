#!/bin/bash
# ==============================================================================
# Tests for bin/lib/git-hooks.sh: which ref a push is judged by, and when the
# tests marker lets bin/pre-push skip the suite. The hook itself is covered by
# bin/lib/pre-push.test.sh.
#
#   bash bin/lib/git-hooks.test.sh
#
# Markers are written in a scratch repository by bin/lib/marker_manifest.py, the
# way bin/run-tests.sh writes them. Self-reports pass/fail, exits non-zero on failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=git-hooks.sh
. "$ROOT/bin/lib/git-hooks.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/git-hooks-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }
same() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (got '$2', want '$3')"; fi; }

echo "== pushed_ref =="
Z=0000000000000000000000000000000000000000
A=1111111111111111111111111111111111111111
B=2222222222222222222222222222222222222222
same "a branch push is named by its remote ref" "$(printf 'refs/heads/worktree-fix+289+x %s refs/heads/fix/289/thing %s\n' "$A" "$Z" | pushed_ref)" "$A fix/289/thing"
same "a delete is skipped for the next update" "$(printf 'refs/heads/x %s refs/heads/old %s\nrefs/heads/y %s refs/heads/new %s\n' "$Z" "$A" "$B" "$Z" | pushed_ref)" "$B new"
same "a tag is skipped" "$(printf 'refs/tags/v1 %s refs/tags/v1 %s\n' "$A" "$Z" | pushed_ref)" ""
same "a main ref listed first does not hide the branch after it" "$(printf 'refs/heads/x %s refs/heads/main %s\nrefs/heads/x %s refs/heads/feat/1/x %s\n' "$A" "$Z" "$B" "$Z" | pushed_ref)" "$B feat/1/x"
same "a main-only push names main" "$(printf 'refs/heads/x %s refs/heads/main %s\n' "$A" "$Z" | pushed_ref)" "$A main"
same "a next-only push names next" "$(printf 'refs/heads/x %s refs/heads/next %s\n' "$A" "$Z" | pushed_ref)" "$A next"
same "a next ref listed first does not hide the branch after it" "$(printf 'refs/heads/x %s refs/heads/next %s\nrefs/heads/x %s refs/heads/feat/1/x %s\n' "$A" "$Z" "$B" "$Z" | pushed_ref)" "$B feat/1/x"
same "main and next together name the first" "$(printf 'refs/heads/x %s refs/heads/next %s\nrefs/heads/x %s refs/heads/main %s\n' "$A" "$Z" "$B" "$Z" | pushed_ref)" "$A next"
same "a branch merely starting with main is not main" "$(printf 'refs/heads/x %s refs/heads/main-2 %s\n' "$A" "$Z" | pushed_ref)" "$A main-2"
same "a branch merely starting with next is not next" "$(printf 'refs/heads/x %s refs/heads/next-up %s\n' "$A" "$Z" | pushed_ref)" "$A next-up"
printf 'refs/heads/a %s refs/heads/feat/1/a %s\nrefs/heads/b %s refs/heads/feat/1/b %s\n' "$A" "$Z" "$B" "$Z" | pushed_ref >/dev/null
same "two branches in one push return 3" "$?" "3"
printf '' | pushed_ref >/dev/null
same "empty stdin returns 1" "$?" "1"

echo "== tests_marker_for =="
M="$WORK/markers"
git init -q "$M"
python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$M" tests "$A"
( cd "$M" && tests_marker_for "$A" ) && ok "a stamped SHA passes" || bad "a stamped SHA passes"
( cd "$M" && tests_marker_for "$B" ) && bad "an unstamped SHA fails" || ok "an unstamped SHA fails"
touch "$M/.claude/markers/tests/$B"
( cd "$M" && tests_marker_for "$B" ) && bad "a marker with no manifest line fails" || ok "a marker with no manifest line fails"
python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$M" review-shipping "$B"
rm -f "$M/.claude/markers/tests/$B"
( cd "$M" && tests_marker_for "$B" ) && bad "a review marker is not a tests marker" || ok "a review marker is not a tests marker"
( cd "$M" && tests_marker_for "" ) && bad "an empty SHA fails" || ok "an empty SHA fails"
same "the check prints nothing" "$(cd "$M" && tests_marker_for "$B" 2>&1; tests_marker_for "$A" 2>&1)" ""
mkdir -p "$WORK/not-a-repo"
( cd "$WORK/not-a-repo" && GIT_CEILING_DIRECTORIES="$WORK" tests_marker_for "$A" ) && bad "outside git fails" || ok "outside git fails"

echo
echo "git-hooks: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
