#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# PostToolUse(ExitPlanMode) reminder: post the Moment 2 comment once a plan is approved.
# Advisory: always exits 0. It reads nothing from the payload, so it has nothing to fail on.
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" 2>/dev/null || exit 0
cat >/dev/null

remind "ISSUE REMINDER, Moment 2 (plan approved): post ONE comment on the issue, at most 600 characters
of body text: what you are building, and any decision that changes what the issue asked for. Not the
file list, test plan or sequencing; those go in the PR. End it with a blank line and then
\"🤖 Drafted by Claude Code\" (docs/github-issues.md § Lifecycle & agent etiquette).
  gh issue comment <n> --repo VATUSA/OIS --body-file <file>
Count the characters before posting. No issue for this work? Skip it; don't open one to comment on."
