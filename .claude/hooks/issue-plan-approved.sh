#!/bin/bash
# PostToolUse hook: remind to post the Moment 2 comment and move the card after a plan is approved.
#
# Trigger: after ExitPlanMode.
# Exit:    always 0. The reminder goes back as additionalContext, the channel a
#          PostToolUse hook has to the agent; plain stdout goes to the debug log.
#
# This is an ADVISORY hook. It protects nothing and writes nothing to GitHub, so
# it must never be the reason a session stalls.
#
# Tests: bin/lib/issue-plan-approved.test.sh
cat >/dev/null
python3 -I - <<'PY' \
    || echo "issue-plan-approved: the reminder could not run; follow github-issues.md -> Moment 2 by hand." >&2
import json
print(json.dumps({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": (
    "ISSUE REMINDER - Moment 2 (Plan Approved). Follow .claude/rules/github-issues.md -> \"Moment 2: Plan Approved\". "
    "Post ONE comment on the issue, 600 characters of body text maximum: what you are building, and any decision "
    "that changes what the issue asked for. NOT the file list, NOT the test plan, NOT the sequencing; those go in "
    "the PR. No attribution of any kind (AGENTS.md -> Git workflow): no footer, no \"Drafted by\" or "
    "\"Generated with\" line, no Co-Authored-By. Write the body to a file, count its characters and check it "
    "first (! grep -qiE 'Drafted by|Generated with|Co-Authored' <file>), then post with: "
    "gh issue comment <n> --repo VATUSA/OIS --body-file <file>. "
    "Then move the card if it is not there yet: bash bin/board-status.sh <n> \"In build\". "
    "If there is no issue for this work, skip both; do not open one just to comment.")}}))
PY
exit 0
