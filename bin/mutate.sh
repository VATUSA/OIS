#!/bin/bash
# ==============================================================================
# Apply one mutation, run a test command against it, restore the file.
#
#   bin/mutate.sh FILE OLD NEW -- COMMAND [ARGS...]
#
# OLD must occur in FILE exactly once (literal text, may span lines). COMMAND
# must pass on the unmutated file first. Prints
#   RED <file>:<line>    COMMAND failed: a test kills the mutation (exit 0)
#   GREEN <file>:<line>  COMMAND passed: nothing kills it (exit 1)
# and the path of COMMAND's output. Exit 2 refuses (bad arguments, a COMMAND that
# fails unmutated, cannot run, is killed by a signal (exit 129-192), or a Rust
# mutant that does not compile); exit 3 means FILE could not be restored.
#
#   bin/mutate.sh backend/src/handlers/tmu.rs 'a < b' 'a <= b' -- \
#       cargo nextest run -p ois-backend -E 'test(tmi_window)'
#   bin/mutate.sh web/src/lib/time.ts 'a < b' 'a <= b' -- \
#       pnpm --filter web exec vitest run src/lib/time.test.ts
#   bin/mutate.sh bin/pre-push 'exit 1' 'exit 0' -- bash bin/lib/pre-push.test.sh
#
# One mutation, one targeted test: name the test with a nextest filter (-E
# 'test(name)') or the vitest file, never the full suite (bin/run-tests.sh is the
# gate). A mutant rustc rejects tells you about the type system, not the tests, so
# a compile error is refused, not counted RED: pick a mutation that type-checks.
# Cargo builds into this checkout's own target/, with sccache when installed
# (bin/lib/portable.sh), so the rebuild after each edit is incremental.
#
# FILE is restored byte for byte on every exit path, interrupts included, from a
# copy taken before the edit (`git checkout --` would also discard unstaged work).
# SIGKILL of this script itself cannot be trapped: the copy stays under
# ${TMPDIR:-/tmp}/mutate.*/original. The mutant is on disk while COMMAND runs, so
# never mutate a tree another run is building (.claude/rules/test-quality.md), and
# mutate .claude/hooks/ only in a scratch copy: every session runs those hooks from
# this checkout. Runs unchanged on macOS (bash 3.2, BSD userland) and Linux.
# Tests: bin/lib/mutate.test.sh.
# ==============================================================================
set -uo pipefail

usage() { echo "usage: bin/mutate.sh FILE OLD NEW -- COMMAND [ARGS...]" >&2; exit 2; }

[ "$#" -ge 5 ] || usage
FILE="$1" OLD="$2" NEW="$3"
[ "$4" = "--" ] || usage
shift 4

[ -f "$FILE" ] || { echo "mutate: $FILE is not a file" >&2; exit 2; }
[ -n "$OLD" ] || { echo "mutate: OLD is empty" >&2; exit 2; }
[ "$OLD" != "$NEW" ] || { echo "mutate: OLD and NEW are the same" >&2; exit 2; }

PORTABLE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/portable.sh"
if [ -f "$PORTABLE" ]; then
    # shellcheck source=lib/portable.sh
    . "$PORTABLE"
    export_build_env
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/mutate.XXXXXX")" || exit 2
BACKUP="$WORK/original"
LOG="$WORK/output.log"
cp -p "$FILE" "$BACKUP" || { echo "mutate: cannot back up $FILE" >&2; exit 2; }

restore() {
    cmp -s "$BACKUP" "$FILE" && return 0
    cp "$BACKUP" "$FILE" && cmp -s "$BACKUP" "$FILE" && return 0
    echo "mutate: FAILED TO RESTORE $FILE; the original is $BACKUP" >&2
    return 1
}

# A kill only counts against a command that passes without the mutation.
"$@" > "$LOG" 2>&1
baseline=$?
if [ "$baseline" -ne 0 ]; then
    echo "mutate: COMMAND fails before any mutation (exit $baseline); fix it first. Output: $LOG" >&2
    exit 2
fi

trap 'restore || exit 3' EXIT

# Replaces the single occurrence and prints its line number; prints the count
# and exits 4 when there is not exactly one.
LINE="$(perl -e '
    my ($path, $old, $new) = @ARGV;
    open(my $in, "<", $path) or exit 3;
    local $/; my $text = <$in>; close $in;
    my $count = () = $text =~ /\Q$old\E/g;
    if ($count != 1) { print $count; exit 4 }
    my $at = index($text, $old);
    my $line = (substr($text, 0, $at) =~ tr/\n//) + 1;
    substr($text, $at, length $old) = $new;
    open(my $out, ">", $path) or exit 3;
    print $out $text; close $out;
    print $line;
' "$FILE" "$OLD" "$NEW")"
case $? in
    0) ;;
    4) echo "mutate: OLD occurs $LINE times in $FILE; it must occur exactly once" >&2
       exit 2 ;;
    *) echo "mutate: cannot edit $FILE" >&2; exit 2 ;;
esac

"$@" > "$LOG" 2>&1
status=$?
# 126/127: the command could not run; 129-192: a signal (an OOM kill) ended it.
if [ "$status" -eq 126 ] || [ "$status" -eq 127 ] || { [ "$status" -ge 129 ] && [ "$status" -le 192 ]; }; then
    echo "mutate: COMMAND could not run or was killed on the mutant (exit $status). Output: $LOG" >&2
    exit 2
fi
# Cargo's build-failure line (rustc rejected the mutant, so no test ran); a failing
# test prints `error: test run failed` instead, which stays RED.
if [ "$status" -ne 0 ] && grep -qE '^error: could not compile ' "$LOG"; then
    echo "mutate: the mutant does not compile (rustc rejected it, no test ran); pick a mutation that type-checks. Output: $LOG" >&2
    exit 2
fi
if [ "$status" -ne 0 ]; then
    echo "RED $FILE:$LINE (output: $LOG)"
    exit 0
fi
echo "GREEN $FILE:$LINE (output: $LOG)"
exit 1
