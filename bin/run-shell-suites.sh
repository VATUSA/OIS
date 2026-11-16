#!/bin/bash
# ==============================================================================
# Runs every bin/lib/*.test.sh suite of the repository the current directory is in,
# one at a time, and fails if any exits non-zero. CI's `shell-tests` job and the
# `shell suites` step of bin/run-tests.sh (`just test`) both call it.
#
#   bash bin/run-shell-suites.sh
#
# Suites are found by glob, never by a list, so a new suite is gated the moment it
# lands. A suite that cannot run on this host says `SKIP <reason>` and exits 0 from a
# check of its own; this runner reads only the exit code. Each suite's output goes to
# a log that is printed only when the suite fails; a passing suite prints just its
# SKIP lines, so a skipped case shows in the run. A summary names every failing suite. No suite found is a failure: a glob that stopped matching must not
# read as a clean run.
#
# Serial on purpose: file-lock and flock-run assert timing windows that a parallel
# run would stretch. stdin is /dev/null, so a suite that reads it ends instead of
# stalling the job.
#
# The suites come from the current directory's repository, not from this script's
# location: bin/lib/run-tests.test.sh runs the real bin/run-tests.sh, and so this
# script, inside scratch repos that hold stub suites.
#
# Tests: bin/lib/run-shell-suites.test.sh
# ==============================================================================
set -uo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)"
if [[ -z "$ROOT" ]]; then
    echo "run-shell-suites: not inside a git repository." >&2
    exit 1
fi
cd "$ROOT" || exit 1

shopt -s nullglob
SUITES=(bin/lib/*.test.sh)
shopt -u nullglob
if [[ ${#SUITES[@]} -eq 0 ]]; then
    echo "run-shell-suites: no suite matches bin/lib/*.test.sh in $ROOT." >&2
    exit 1
fi

LOG="$(mktemp "${TMPDIR:-/tmp}/run-shell-suites.XXXXXX")"
trap 'rm -f "$LOG"' EXIT

PASSED=0
FAILED=()
for suite in "${SUITES[@]}"; do
    bash "$suite" </dev/null >"$LOG" 2>&1
    rc=$?
    if [[ $rc -eq 0 ]]; then
        echo "PASS $suite"
        sed -nE 's/^[[:space:]]*(SKIP.*)$/    \1/p' "$LOG"
        PASSED=$((PASSED + 1))
    else
        echo "FAIL $suite (exit $rc)"
        sed 's/^/    /' "$LOG"
        FAILED+=("$suite")
    fi
done

echo
echo "shell suites: $PASSED passed, ${#FAILED[@]} failed"
if [[ ${#FAILED[@]} -gt 0 ]]; then
    printf -v names '%s, ' "${FAILED[@]}"
    echo "failing: ${names%, }"
    exit 1
fi
