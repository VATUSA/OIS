#!/bin/bash
# ==============================================================================
# Tests for bin/mutate.sh.
#
#   bash bin/lib/mutate.test.sh
#
# Mutates throwaway files under a temp dir; pure shell, runs unchanged on macOS
# and Linux. Self-reports pass/fail and exits non-zero if anything failed.
# ==============================================================================
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$DIR/../mutate.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/mutate-test.XXXXXX")"
cleanup() { chmod -R u+w "$WORK" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

F="$WORK/guard.rs"
fixture() { chmod u+w "$F" 2>/dev/null; printf 'line one\nif ($a < $b) {\n    return 1;\n}\nno newline at end' > "$F"; cp "$F" "$WORK/expected"; }
unchanged() { cmp -s "$F" "$WORK/expected"; }

# expect NAME WANT_EXIT WANT_PREFIX OUT EXIT
expect() {
    if [ "$5" -eq "$2" ] && case "$4" in "$3"*) true ;; *) false ;; esac; then ok "$1"; else bad "$1 (exit $5, output: $4)"; fi
}

echo "== verdicts =="
fixture
out="$(bash "$SCRIPT" "$F" '$a < $b' '$a <= $b' -- grep -q 'line one' "$F" 2>&1)"; rc=$?
expect "a command passing on the mutant is GREEN with the mutated line" 1 "GREEN $F:2" "$out" "$rc"
unchanged && ok "file restored after GREEN" || bad "file not restored after GREEN"

fixture
out="$(bash "$SCRIPT" "$F" '$a < $b' '$a <= $b' -- grep -q '< \$b' "$F" 2>&1)"; rc=$?
expect "a command failing only on the mutant is RED" 0 "RED $F:2" "$out" "$rc"
unchanged && ok "file restored after RED" || bad "file not restored after RED"

fixture
out="$(bash "$SCRIPT" "$F" 'at end' 'AT END' -- grep -q 'at end' "$F" 2>&1)"; rc=$?
expect "the command sees the mutation" 0 "RED $F:5" "$out" "$rc"
unchanged && ok "a missing trailing newline survives the restore" || bad "trailing bytes changed"

fixture
out="$(bash "$SCRIPT" "$F" "    return 1;
}" '}' -- grep -q 'return 1' "$F" 2>&1)"; rc=$?
expect "OLD may span lines" 0 "RED $F:3" "$out" "$rc"
unchanged && ok "multi-line mutation restored" || bad "multi-line mutation not restored"

echo "== a kill only counts against a command that passes unmutated =="
fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- false 2>&1)"; rc=$?
expect "a command failing before any mutation is refused" 2 "mutate: COMMAND fails before any mutation (exit 1)" "$out" "$rc"
unchanged && ok "untouched when the baseline fails" || bad "touched when the baseline fails"

fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && exit 127; exit 0' "$F" 2>&1)"; rc=$?
expect "a command that cannot run on the mutant is refused, not RED" 2 "mutate: COMMAND could not run or was killed on the mutant (exit 127)" "$out" "$rc"
unchanged && ok "restored after a command that could not run" || bad "not restored after exit 127"

fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && kill -KILL $$; exit 0' "$F" 2>&1)"; rc=$?
# bash prints its own "Killed" line first, so match the refusal anywhere.
expect "a run killed by a signal is refused, not RED" 2 "" "$rc" "$rc"
case "$out" in *"mutate: COMMAND could not run or was killed on the mutant (exit 137)"*) ok "  and says it was killed" ;; *) bad "  killed message: $out" ;; esac
unchanged && ok "restored after a killed run" || bad "not restored after a killed run"

fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && exit 255; exit 0' "$F" 2>&1)"; rc=$?
expect "a panicking test's 101 on the mutant is RED" 0 "RED $F:1" "$out" "$rc"
fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && { echo "error: test run failed"; exit 100; }; exit 0' "$F" 2>&1)"; rc=$?
expect "nextest's failed-run line on the mutant is RED" 0 "RED $F:1" "$out" "$rc"
fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && { echo "error[E0308]: mismatched types"; echo "error: could not compile \`ois-backend\` (lib test) due to 1 previous error"; exit 101; }; exit 0' "$F" 2>&1)"; rc=$?
expect "a mutant rustc rejects is refused, not RED" 2 "mutate: the mutant does not compile" "$out" "$rc"
unchanged && ok "restored after a mutant that does not compile" || bad "not restored after a compile error"
for code in 128 193; do
    fixture
    out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c "grep -q 'line two' \"\$0\" && exit $code; exit 0" "$F" 2>&1)"; rc=$?
    expect "exit $code on the mutant is RED, not a signal" 0 "RED $F:1" "$out" "$rc"
done

for code in 129 192; do
    fixture
    out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c "grep -q 'line two' \"\$0\" && exit $code; exit 0" "$F" 2>&1)"; rc=$?
    expect "exit $code on the mutant is refused as a signal" 2 "mutate: COMMAND could not run or was killed on the mutant (exit $code)" "$out" "$rc"
done

fixture
chmod a-w "$F"
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- true 2>&1)"; rc=$?
expect "a file perl cannot write is refused" 2 "mutate: cannot edit $F" "$out" "$rc"
chmod u+w "$F"
unchanged && ok "untouched when it cannot be edited" || bad "touched when it cannot be edited"

printf 'exit 0\n' > "$WORK/noexec.sh"
fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && exec "$1"; exit 0' "$F" "$WORK/noexec.sh" 2>&1)"; rc=$?
expect "a command that is not executable (126) is refused" 2 "mutate: COMMAND could not run or was killed on the mutant (exit 126)" "$out" "$rc"

echo "== refusals leave the file untouched =="
fixture
out="$(bash "$SCRIPT" "$F" 'absent' 'x' -- true 2>&1)"; rc=$?
expect "OLD absent is refused" 2 "mutate: OLD occurs 0 times" "$out" "$rc"
unchanged && ok "untouched when OLD is absent" || bad "touched when OLD is absent"

fixture
out="$(bash "$SCRIPT" "$F" 'e' 'E' -- true 2>&1)"; rc=$?
expect "OLD occurring twice is refused" 2 "mutate: OLD occurs" "$out" "$rc"
unchanged && ok "untouched when OLD repeats" || bad "touched when OLD repeats"

fixture
out="$(bash "$SCRIPT" "$F" '' 'x' -- true 2>&1)"; rc=$?
expect "empty OLD is refused" 2 "mutate: OLD is empty" "$out" "$rc"

out="$(bash "$SCRIPT" "$F" 'line one' 'line one' -- true 2>&1)"; rc=$?
expect "OLD equal to NEW is refused" 2 "mutate: OLD and NEW are the same" "$out" "$rc"

out="$(bash "$SCRIPT" "$F" 'line one' 'line two' true 2>&1)"; rc=$?
expect "too few arguments are refused" 2 "usage:" "$out" "$rc"

out="$(bash "$SCRIPT" "$F" 'line one' 'line two' X true 2>&1)"; rc=$?
expect "a fourth argument other than -- is refused" 2 "usage:" "$out" "$rc"
unchanged && ok "untouched when -- is missing" || bad "touched when -- is missing"

out="$(bash "$SCRIPT" "$WORK/nope.rs" 'a' 'b' -- true 2>&1)"; rc=$?
expect "a missing file is refused" 2 "mutate: $WORK/nope.rs is not a file" "$out" "$rc"

echo "== the command builds into the checkout's own target dir =="
REPO="$WORK/repo"
git init -q "$REPO"
fixture
out="$(cd "$REPO" && env -u CARGO_TARGET_DIR bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'echo "target=${CARGO_TARGET_DIR-unset}" > "$1"; exit 0' "$F" "$WORK/target.seen" 2>&1)"; rc=$?
[ "$(cat "$WORK/target.seen")" = "target=unset" ] && ok "no CARGO_TARGET_DIR is exported (cargo uses the checkout's target/)" || bad "CARGO_TARGET_DIR seen: $(cat "$WORK/target.seen")"
out="$(cd "$REPO" && CARGO_TARGET_DIR=/elsewhere bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'echo "target=$CARGO_TARGET_DIR" > "$1"; exit 0' "$F" "$WORK/target.seen" 2>&1)"; rc=$?
[ "$(cat "$WORK/target.seen")" = "target=/elsewhere" ] && ok "a CARGO_TARGET_DIR the caller set is kept" || bad "CARGO_TARGET_DIR seen: $(cat "$WORK/target.seen")"

echo "== restore =="
fixture
out="$(bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && chmod a-w "$0"; exit 0' "$F" 2>&1)"; rc=$?
if [ "$rc" -eq 3 ] && case "$out" in *"FAILED TO RESTORE $F"*) true ;; *) false ;; esac; then ok "a failed restore exits 3 and says where the original is"; else bad "failed restore (exit $rc): $out"; fi
chmod u+w "$F"

fixture
bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && sleep 30; exit 0' "$F" > /dev/null 2>&1 &
pid=$!
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do grep -q 'line two' "$F" && break; sleep 0.2; done
grep -q 'line two' "$F" && ok "mutation applied before the interrupt" || bad "mutation never applied"
kill -TERM "$pid"
wait "$pid" 2>/dev/null
unchanged && ok "file restored after SIGTERM" || bad "file not restored after SIGTERM"

echo
echo "mutate: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
