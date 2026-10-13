#!/bin/bash
# PreToolUse hook (Bash, Write, Edit, MultiEdit, NotebookEdit): block hand-written
# tests, review-shipping and code-review markers.
#
# The markers are the proof the PR gates read: a file named for a commit says the suite
# or the review ran on it. Only bin/run-tests.sh and bin/review-marker.sh write them,
# after checking that it did; a marker written any other way is a forged pass. The
# lenses' attestations (.claude/markers/attest/) are written by hand and stay allowed.
# lib/marker_write.py refuses the literal spellings it can read; lib/marker_ledger.py then
# records what the call may stamp, which marker-audit.sh holds the outcome against.
#
# Blocking contract: only `exit 2` blocks, and Claude Code reads stderr then. Any
# failure to run the check (python3 missing, unreadable payload) blocks.
#
# Tests: bin/lib/marker-write-gate.test.sh
exec >&2

INPUT=$(cat)

LIB="$(dirname "${BASH_SOURCE[0]}")/lib"
REASON="$(printf '%s' "$INPUT" | python3 -I "$LIB/marker_write.py" 2>/dev/null)"
if [[ $? -ne 0 ]]; then
    echo "BLOCKED: the marker-write check could not run, so a marker write cannot be ruled out."
    exit 2
fi
if [[ -n "$REASON" ]]; then
    echo "BLOCKED: tests and review markers are written only by bin/run-tests.sh and bin/review-marker.sh ($REASON)."
    echo "Run the suite (just test) or finish the review (bash bin/review-marker.sh ...) instead."
    exit 2
fi
# Record what this call may stamp, for marker-audit.sh to hold its manifest changes against.
if ! printf '%s' "$INPUT" | python3 -I "$LIB/marker_ledger.py" snapshot 2>/dev/null; then
    echo "BLOCKED: the marker ledger could not record this call, so a marker it writes could not be told apart"
    echo "from a forged one."
    exit 2
fi
exit 0
