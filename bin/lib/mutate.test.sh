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
ok()   { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad()  { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

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
# The failed restore keeps its mutate.* dir by design; keep it inside $WORK.
mkdir -p "$WORK/tmp.restore"
fixture
out="$(TMPDIR="$WORK/tmp.restore" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && chmod a-w "$0"; exit 0' "$F" 2>&1)"; rc=$?
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

echo "== the work directory goes with the run (#1020) =="
# Each case runs the script with TMPDIR="$WORK/tmp" (an empty dir made for it), so
# `find "$WORK/tmp" -maxdepth 1 -name 'mutate.*'` counts exactly the dirs this run left.
#
# Coverage matrix (VATUSA/OIS#1020, AC3)
#
# Requirement                              | Case                                                   | Status
# -----------------------------------------|--------------------------------------------------------|-------
# removed after RED                        | no mutate.* dir remains after RED                      | done
# removed after GREEN                      | no mutate.* dir remains after GREEN                    | done
# removed after a baseline refusal         | no mutate.* dir remains after a baseline failure       | done
# removed after a compile-error refusal    | no mutate.* dir remains after a mutant that won't build| done
# removed after an OLD-count refusal       | no mutate.* dir remains when OLD occurs 0 or 2 times   | done
# removed after the other refusals         | ...after 126/127/signal exits and an uneditable FILE   | done
# removed after an interrupt               | no mutate.* dir remains after SIGTERM / SIGINT         | done
# kept when FILE could not be restored     | a failed restore (exit 3) leaves the dir and original  | done
# the log path is not printed (implied)    | RED and GREEN lines carry no "output:"                 | done
# a refusal still shows the output         | a refusal prints the last 40 log lines to stderr       | done
#                                          | a refusal names no path the cleanup removed            | done

# newtmp NAME: an empty TMPDIR for one case; left COUNTS the mutate.* dirs a run left in it.
newtmp() { T="$WORK/tmp.$1"; mkdir -p "$T"; }
left() { find "$T" -mindepth 1 -maxdepth 1 -name 'mutate.*' | wc -l | tr -d ' '; }
none_left() { [ "$(left)" = 0 ] && ok "$1" || bad "$1 ($(left) left: $(ls "$T"))"; }

newtmp red; fixture
out="$(TMPDIR="$T" bash "$SCRIPT" "$F" '$a < $b' '$a <= $b' -- grep -q '< \$b' "$F" 2>&1)"; rc=$?
expect "RED under a fresh TMPDIR" 0 "RED $F:2" "$out" "$rc"
none_left "no mutate.* dir remains after RED"
[ "$out" = "RED $F:2" ] && ok "the RED line carries no output: path" || bad "the RED line carries no output: path ($out)"

newtmp green; fixture
out="$(TMPDIR="$T" bash "$SCRIPT" "$F" '$a < $b' '$a <= $b' -- grep -q 'line one' "$F" 2>&1)"; rc=$?
expect "GREEN under a fresh TMPDIR" 1 "GREEN $F:2" "$out" "$rc"
none_left "no mutate.* dir remains after GREEN"
[ "$out" = "GREEN $F:2" ] && ok "the GREEN line carries no output: path" || bad "the GREEN line carries no output: path ($out)"

newtmp baseline; fixture
out="$(TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'i=1; while [ $i -le 50 ]; do printf "L%02d\n" $i; i=$((i + 1)); done; exit 1' 2>&1)"; rc=$?
expect "a baseline failure is refused" 2 "mutate: COMMAND fails before any mutation (exit 1)" "$out" "$rc"
none_left "no mutate.* dir remains after a baseline failure"
case "$out" in *L11*L50*) ok "a refusal prints the 40th-from-last line through the last" ;; *) bad "a refusal prints the last 40 lines: $out" ;; esac
case "$out" in *L10*) bad "a refusal prints only the last 40 lines (L10 present)" ;; *) ok "a refusal prints only the last 40 lines" ;; esac
case "$out" in *"$T"*) bad "a refusal names no path the cleanup removed: $out" ;; *) ok "a refusal names no path the cleanup removed" ;; esac

newtmp compile; fixture
out="$(TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && { echo "error[E0308]: mismatched types"; echo "error: could not compile \`ois-backend\` (lib test) due to 1 previous error"; exit 101; }; exit 0' "$F" 2>&1)"; rc=$?
expect "a mutant that does not compile is refused" 2 "mutate: the mutant does not compile" "$out" "$rc"
none_left "no mutate.* dir remains after a mutant that does not compile"
case "$out" in *"error[E0308]: mismatched types"*) ok "the compile refusal shows rustc's error" ;; *) bad "the compile refusal shows rustc's error: $out" ;; esac

for old in absent e; do
    newtmp "old-$old"; fixture
    out="$(TMPDIR="$T" bash "$SCRIPT" "$F" "$old" 'X' -- true 2>&1)"; rc=$?
    expect "OLD '$old' is refused" 2 "mutate: OLD occurs" "$out" "$rc"
    none_left "no mutate.* dir remains when OLD '$old' does not occur exactly once"
done

for code in 126 127 129; do
    newtmp "exit-$code"; fixture
    out="$(TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c "grep -q 'line two' \"\$0\" && { echo 'cmd says why'; exit $code; }; exit 0" "$F" 2>&1)"; rc=$?
    expect "exit $code on the mutant is refused" 2 "mutate: COMMAND could not run or was killed on the mutant (exit $code)" "$out" "$rc"
    none_left "no mutate.* dir remains after exit $code on the mutant"
    case "$out" in *"cmd says why"*) ok "the exit-$code refusal shows the command's output" ;; *) bad "the exit-$code refusal shows the command's output: $out" ;; esac
done

newtmp readonly; fixture; chmod a-w "$F"
out="$(TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- true 2>&1)"; rc=$?
expect "an uneditable FILE is refused" 2 "mutate: cannot edit $F" "$out" "$rc"
none_left "no mutate.* dir remains when FILE cannot be edited"
chmod u+w "$F"

newtmp sigterm; fixture
TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && sleep 2; exit 0' "$F" > /dev/null 2>&1 &
pid=$!
for _ in $(seq 1 50); do grep -q 'line two' "$F" && break; sleep 0.1; done
grep -q 'line two' "$F" && ok "the mutation is on disk before SIGTERM" || bad "the mutation is on disk before SIGTERM"
kill -TERM "$pid"; wait "$pid" 2>/dev/null; rc=$?
[ "$rc" -eq 143 ] && ok "the script dies of SIGTERM (exit 143)" || bad "the script dies of SIGTERM (exit 143, got $rc)"
unchanged && ok "FILE restored after SIGTERM" || bad "FILE restored after SIGTERM"
none_left "no mutate.* dir remains after SIGTERM during COMMAND"

# `cmd &` in a non-interactive shell starts with SIGINT ignored, which bash cannot trap;
# job control (set -m) starts it in its own process group with SIGINT at its default.
newtmp sigint; fixture
set -m
TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && sleep 2; exit 0' "$F" > /dev/null 2>&1 &
pid=$!
set +m
for _ in $(seq 1 50); do grep -q 'line two' "$F" && break; sleep 0.1; done
grep -q 'line two' "$F" && ok "the mutation is on disk before SIGINT" || bad "the mutation is on disk before SIGINT"
# Ctrl-C signals the whole process group, COMMAND included.
kill -INT -- "-$pid"; wait "$pid" 2>/dev/null; rc=$?
[ "$rc" -eq 130 ] && ok "the script dies of SIGINT (exit 130)" || bad "the script dies of SIGINT (exit 130, got $rc)"
unchanged && ok "FILE restored after SIGINT" || bad "FILE restored after SIGINT"
none_left "no mutate.* dir remains after SIGINT during COMMAND"

newtmp backup; fixture; chmod 000 "$F"
if [ "$(id -u)" -eq 0 ]; then
    echo "  SKIP: running as root, which chmod 000 does not stop"
else
    out="$(TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- true 2>&1)"; rc=$?
    # cp prints its own error first, so match the refusal anywhere.
    case "$rc:$out" in "2:"*"mutate: cannot back up $F"*) ok "a FILE that cannot be backed up is refused" ;; *) bad "a FILE that cannot be backed up is refused (exit $rc): $out" ;; esac
    none_left "no mutate.* dir remains when FILE cannot be backed up"
fi
chmod 644 "$F"

# Before the edit the trap must not restore: FILE can only differ from the copy then
# because something else wrote it, and the copy would overwrite that write.
newtmp preedit; fixture
out="$(TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'echo appended >> "$0"; exit 1' "$F" 2>&1)"; rc=$?
expect "a baseline that writes FILE is refused" 2 "mutate: COMMAND fails before any mutation" "$out" "$rc"
grep -q 'appended' "$F" && ok "a refusal before the edit leaves FILE as something else wrote it" || bad "a refusal before the edit leaves FILE as something else wrote it"
none_left "no mutate.* dir remains after that refusal"

newtmp restorefail; fixture
out="$(TMPDIR="$T" bash "$SCRIPT" "$F" 'line one' 'line two' -- sh -c 'grep -q "line two" "$0" && chmod a-w "$0"; exit 0' "$F" 2>&1)"; rc=$?
chmod u+w "$F"
[ "$rc" -eq 3 ] && ok "a failed restore exits 3" || bad "a failed restore exits 3 (got $rc): $out"
[ "$(left)" = 1 ] && ok "a failed restore leaves its mutate.* dir" || bad "a failed restore leaves its mutate.* dir ($(left))"
KEPT="$(find "$T" -mindepth 1 -maxdepth 1 -name 'mutate.*' | head -1)"
cmp -s "$KEPT/original" "$WORK/expected" && ok "...holding the original byte for byte" || bad "...holding the original byte for byte"
case "$out" in *"the original is $KEPT/original"*) ok "...at the path the message names" ;; *) bad "...at the path the message names: $out" ;; esac

echo
echo "mutate: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
