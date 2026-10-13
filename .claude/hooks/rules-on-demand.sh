#!/bin/bash
# PreToolUse on EnterWorktree and Bash: the rule files named as arguments carry paths: frontmatter,
# so they are not loaded at startup. Remind the agent to read them before entering a worktree, and
# before a Bash line that runs `gh issue`, `gh project` or `gh pr` (parsed by command_match.py, so a
# heredoc or quoted argument naming them stays quiet). Advisory: always exit 0. The payload is read
# from stdin, never argv or the environment, so a large command cannot fail exec; if the parse
# fails the reminder fires anyway (fail open).
# Tests: bin/lib/rules-on-demand.test.sh.

PROG=$(cat <<'PY'
import json, os, sys

sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
files = " ".join(sys.argv[2:])
GH = (["gh", "issue"], ["gh", "project"], ["gh", "pr"])

def remind():
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext":
        "RULES: not loaded at startup. Unless you have read them in full since the session started "
        "or was last summarized, read in full now: " + files}}))
    sys.exit(0)

try:
    payload = json.loads(sys.stdin.read() or "{}")
except ValueError:
    remind()
if payload.get("tool_name") != "Bash":
    remind()
command = (payload.get("tool_input") or {}).get("command") or ""
try:
    from command_match import Unparseable, invokes
except Exception:
    remind()
try:
    if any(invokes(command, target) for target in GH):
        remind()
except Unparseable:
    remind()
PY
)

python3 -I -c "$PROG" "$(dirname "${BASH_SOURCE[0]}")/lib" "$@" \
    || printf '{"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "RULES: not loaded at startup. Unless you have read them in full since the session started or was last summarized, read in full now: %s"}}\n' "$*"
exit 0
