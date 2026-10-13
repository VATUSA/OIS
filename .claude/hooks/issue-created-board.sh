#!/bin/bash
# PostToolUse hook: an issue that is not on the board is not work.
#
# Trigger: after a Bash `gh issue create`.
# Exit:    always 0 — ADVISORY, never a gate. It runs after the issue already
#          exists, so there is nothing left to block; it returns the second half
#          of the job, with the issue number filled in, as additionalContext (plain
#          stdout from a PostToolUse hook never reaches the agent).
#
# Why it exists: `gh issue create` puts nothing on Project 7. `gh project
# item-add` then leaves Status empty, so the item sits in no column — filed,
# and invisible on the thing people actually read. Both steps get missed
# because the first one prints a URL and looks like success.
#
# The line is parsed (command_match.py), so a `cat` or heredoc naming the command
# stays quiet; an unparseable line still fires. It fails OPEN by design: a
# reminder that could block a Bash call would be worse than one that misses.
#
# Tests: bin/lib/issue-created-board.test.sh

# The payload is read from stdin, never the environment: a large tool response in an
# environment variable fails exec (E2BIG) before the reminder runs.
IFS= read -r -d '' PROG <<'PY' || true
import json, re, sys

sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
from command_match import Unparseable, invokes

try:
    payload = json.loads(sys.stdin.read() or "{}")
except ValueError:
    sys.exit(0)
if payload.get("tool_name") != "Bash":
    sys.exit(0)
command = (payload.get("tool_input") or {}).get("command") or ""
try:
    created = invokes(command, ["gh", "issue", "create"])
except Unparseable:
    created = True
if not created:
    sys.exit(0)

# The issue number, read back from what gh printed. Best effort: the reminder is
# still correct without it.
result = payload.get("tool_response", payload.get("tool_result", ""))
if isinstance(result, dict):
    result = result.get("stdout", result.get("content", ""))
found = re.findall(r"/issues/(\d+)", str(result))
n = found[-1] if found else "<n>"

reminder = """BOARD REMINDER - the issue is not on the Kanban yet.

Filing is TWO steps (.claude/rules/github-issues.md -> "Filing an issue is TWO steps").
`gh issue create` did the first. Do the second NOW, before you report back to the user:

  gh project item-add 7 --owner VATUSA --url https://github.com/VATUSA/OIS/issues/{n}

  bash bin/board-status.sh {n} "Triaging"

Skipping the move leaves the item in NO column. item-add prints nothing on success, and a
printed "moved" line is not the resource: read the status back with the projectItems query in
github-issues.md -> "Referring to an issue" before you tell the user the issue is filed. An
"is not on project 7" straight after item-add is lag: re-run the move, never the item-add.
Never list the board to find the item (`gh project item-list` trips the Projects secondary
rate limit); board-status.sh reads it from the issue's own projectItems.
"Triaging" is where your involvement ends: accepting and prioritizing are for the user to decide.""".format(n=n)
print(json.dumps({"hookSpecificOutput": {"hookEventName": "PostToolUse",
                                         "additionalContext": reminder}}))
PY

python3 -I -c "$PROG" "$(dirname "${BASH_SOURCE[0]}")/lib" \
    || echo "issue-created-board: the reminder could not run; follow github-issues.md -> \"Filing an issue is TWO steps\" by hand." >&2
exit 0
