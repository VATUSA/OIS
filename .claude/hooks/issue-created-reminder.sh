#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PostToolUse(Bash) reminder: an issue that is not on the board is not work.
#
# `gh issue create` puts nothing on Project 7, and an added item has no Status, so it sits in no
# column. Both steps get missed because the first one prints a URL and looks finished
# (docs/github-issues.md § Lifecycle: new issues start in Triaging).
# Advisory: always exits 0, and stays quiet on a payload it cannot read.
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" 2>/dev/null || exit 0

INPUT="$(cat)"
COMMAND="$(printf '%s' "$INPUT" | hook_json '.tool_input.command // empty')"
[[ "$COMMAND" == *issue* ]] || exit 0

FOUND=0
inspect() { gh_is issue create && FOUND=1; return 0; }
for_each_command inspect
[[ $FOUND -eq 1 ]] || exit 0

# The issue URL gh printed, if the response carries it.
URL="$(printf '%s' "$INPUT" | hook_json '.tool_response | if type == "object" then (.stdout // "") else tostring end' |
    grep -oE 'https://github\.com/[^ ]+/issues/[0-9]+' | tail -1)"
N="${URL##*/}"
[[ -n "$URL" ]] || { URL="<issue-url>"; N="<n>"; }

remind "BOARD REMINDER: issue $N is not on Project 7 yet. Do this now, before reporting it filed:
  gh project item-add 7 --owner VATUSA --url $URL
  .claude/scripts/board-status.sh $N \"Triaging\"
New issues start in Triaging (docs/github-issues.md § Lifecycle); triage and priority are a human's call.
board-status.sh prints \"moved #$N → Triaging\" on success: read that line, don't assume it."
