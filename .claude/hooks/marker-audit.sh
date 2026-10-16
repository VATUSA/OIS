#!/bin/bash
# PostToolUse hook (Bash, Write, Edit, MultiEdit, NotebookEdit) and PostToolUseFailure hook (Bash): keep
# only the manifest lines a stamping call accounts for, and delete every tests, review-shipping or
# code-review marker the manifest does not vouch for, in every worktree.
#
# marker-write-gate.sh refuses the spellings it can read before a tool runs and records what the call
# may stamp; this checks the outcome after it, whatever wrote the files (lib/marker_ledger.py). A line
# is kept only by the audit of the call that may stamp it (`just test`, bin/run-tests.sh or
# bin/review-marker.sh on a line that is nothing but that command and reads) or by a backgrounded one's
# one-shot grant. A line only another call's open snapshot could explain is held out of the manifest
# until that call's own audit; any other is reverted, and the hook returns a block decision and
# additionalContext saying what it undid. The failure event closes a failed stamper's snapshot at once.
# A PostToolUse hook cannot undo anything else the tool did; check_marker_for_head refuses an unvouched
# marker at PR time regardless.
#
# Residual: a guard against an agent's mistake or shortcut. A deliberate multi-step forgery of the
# ledger's own state under the git common dir is outside it.
#
# Exit 0 with JSON on stdout; exit 2 (stderr shown to Claude) when the audit cannot run.
#
# Tests: bin/lib/marker-audit.test.sh

if ! python3 -I "$(dirname "${BASH_SOURCE[0]}")/lib/marker_ledger.py" audit; then
    echo "marker-audit: the marker audit could not run, so a hand-written marker or manifest line may still" \
         "be on disk; the PR gates still refuse any marker without its manifest line." >&2
    exit 2
fi
exit 0
