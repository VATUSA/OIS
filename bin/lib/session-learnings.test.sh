#!/bin/bash
# ==============================================================================
# Tests for bin/session-learnings.sh.
#
#   bash bin/lib/session-learnings.test.sh
#
# Works in a temp directory (SESSION_LEARNINGS_DIR), never /tmp's real files.
# Pure shell; runs unchanged on macOS and Linux. Self-reports pass/fail, exits
# non-zero on any failure.
# ==============================================================================
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$DIR/../session-learnings.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/session-learnings-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export SESSION_LEARNINGS_DIR="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }
DAY="$(date +%Y-%m-%d)"

echo "== new =="
first="$(bash "$SCRIPT" new)"
[[ "$first" == "$WORK/claude-session-learnings-$DAY-1" && -f "$first" ]] && ok "the first file today is -1" || bad "first: $first"
second="$(bash "$SCRIPT" new)"
[[ "$second" == "$WORK/claude-session-learnings-$DAY-2" ]] && ok "a second session gets -2" || bad "second: $second"
: > "$WORK/claude-session-learnings-$DAY-4"
third="$(bash "$SCRIPT" new)"
[[ "$third" == "$WORK/claude-session-learnings-$DAY-3" ]] && ok "the lowest free number is used" || bad "third: $third"
printf 'kept\n' > "$first"
bash "$SCRIPT" new > /dev/null
[[ "$(cat "$first")" == "kept" ]] && ok "an existing file is never truncated" || bad "existing file changed"

bash "$SCRIPT" new extra > /dev/null 2>&1
[[ $? -eq 2 ]] && ok "new takes no arguments" || bad "new accepted an argument"
mkdir -p "$WORK/full"
i=1; while [ "$i" -le 999 ]; do : > "$WORK/full/claude-session-learnings-$DAY-$i"; i=$((i + 1)); done
rm "$WORK/full/claude-session-learnings-$DAY-999"
got="$(SESSION_LEARNINGS_DIR="$WORK/full" bash "$SCRIPT" new 2>&1)"
[[ "$got" == "$WORK/full/claude-session-learnings-$DAY-999" ]] && ok "the 999th file of a day is still created" || bad "999th: $got"
out="$(SESSION_LEARNINGS_DIR="$WORK/full" bash "$SCRIPT" new 2>&1)"; rc=$?
[[ $rc -eq 1 && "$out" == *"cannot create a file"* ]] && ok "a day with 999 files refuses a 1000th" || bad "cap (rc=$rc): $out"

echo "== concurrent sessions never share a file =="
for i in 1 2 3 4 5 6; do bash "$SCRIPT" new > "$WORK/race.$i" & done
wait
dupes="$(cat "$WORK"/race.* | sort | uniq -d)"
[[ -z "$dupes" && "$(cat "$WORK"/race.* | grep -c .)" -eq 6 ]] && ok "six parallel creates get six files" || bad "duplicates: $dupes"

echo "== add =="
out="$(bash "$SCRIPT" add "$second" '#1 review: `foo` broke' 'Per-call state must be cleared on every exit path' 'code-quality.md: extend "Fail-Open"' 2>&1)"; rc=$?
body="$(cat "$second")"
if [[ $rc -eq 0 && "$body" == "===================="*"===================="$'\n''Context: #1 review: `foo` broke'$'\n''Learning: Per-call state must be cleared on every exit path'$'\n''Fix: code-quality.md: extend "Fail-Open"' ]]; then
    ok "an entry is fenced and keeps its text verbatim"
else
    bad "entry (rc=$rc, $out): $body"
fi
bash "$SCRIPT" add "$second" c l f > /dev/null
[[ "$(grep -c '^====================' "$second")" -eq 2 ]] && ok "entries append" || bad "append: $(cat "$second")"

before="$(cat "$second")"
for args in "add $second c l" "add $second c '' f" "add $WORK/other.txt c l f" "add $WORK/claude-session-learnings-$DAY-99 c l f" "bogus"; do
    eval "bash \"\$SCRIPT\" $args" > /dev/null 2>&1; rc=$?
    [[ $rc -eq 2 && "$(cat "$second")" == "$before" ]] && ok "refused without writing: ${args//$WORK/WORK}" || bad "accepted: $args (rc=$rc)"
done
printf 'notes\n' > "$WORK/notes.txt"
bash "$SCRIPT" add "$WORK/notes.txt" c l f > /dev/null 2>&1; rc=$?
[[ $rc -eq 2 && "$(cat "$WORK/notes.txt")" == "notes" ]] && ok "an existing file \`new\` did not make is never appended to" || bad "appended to notes.txt (rc=$rc)"
[[ ! -e "$WORK/other.txt" &&! -e "$WORK/claude-session-learnings-$DAY-99" ]] && ok "a refused add creates no file" || bad "a refused add created a file"

echo
echo "session-learnings: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
