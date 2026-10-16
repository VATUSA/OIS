#!/bin/bash
# PreToolUse hook: remind to run Moment 3 when a PR is opened.
#
# Trigger: before a Bash `gh pr create`.
# Exit:    always 0 — ADVISORY, never a gate. The PR-blocking decision belongs to
#          pre-pr-gate.sh; this hook only prints what happens next.
#
# The reminder goes back as additionalContext: plain stdout from a PreToolUse hook,
# and stderr on exit 0, never reach the agent. The line is parsed (command_match.py),
# so a `cat` or heredoc naming `gh pr create` stays quiet while `cd x && gh pr create`
# fires; an unparseable line fires too. It fails OPEN: a reminder that could block
# gh pr create would be worse than one that occasionally misses.
#
# Tests: bin/lib/issue-work-complete.test.sh

# The payload is read from stdin, never the environment: a large command in an
# environment variable fails exec (E2BIG) before the reminder runs.
IFS= read -r -d '' PROG <<'PY' || true
import json, sys

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
    opened = invokes(command, ["gh", "pr", "create"])
except Unparseable:
    opened = True
if not opened:
    sys.exit(0)

reminder = """ISSUE REMINDER - Moment 3 (Work Complete). Once this PR exists, follow .claude/rules/github-issues.md -> "Moment 3".

0. Read the PR number from gh's output; never predict it. Prove the push first:
   git ls-remote origin refs/heads/<branch> must print git rev-parse HEAD.

1. Post the verification comment, 1,200 characters of body text maximum; count them before posting:
     Done: <one sentence>. PR #<n>.
     How to check: numbered steps, each naming a REAL route path and a REAL control
     Edge cases: at most three, each one this branch actually handles
     Blast radius / Data path (the PR's diagram) / Deploy: migrations, client regen, env, or "nothing"
   Every line must trace to code in THIS diff. Page paths come from web/src/router.tsx and API
   paths from backend/src/router.rs, never invented. Name the path a step needs: a national-scope
   and a facility-scope principal are two paths, and each trajectory caller is its own path.
   Delete any line you cannot validate against the diff.
   No attribution of any kind (AGENTS.md -> Git workflow): no footer, no "Drafted by" or
   "Generated with" line, no Co-Authored-By. Check the body file before posting:
     ! grep -qiE 'Drafted by|Generated with|Co-Authored' <file>

2. Move the board item to "Post build": bash bin/board-status.sh <n> "Post build". NOT to
   "Code Review", "Testing Queue", "Shippable" or "Done": review, verification and merging are the user's call.

3. Follow-up issues ONLY where all four scope tests pass (technical debt or defect +
   pre-existing on origin/next + not work the issue asks for + not already filed). Search the SYMBOL with
   --state all before filing anything.

Do NOT set a priority label yourself. Put the options to the user and let them pick."""
print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse",
                                         "additionalContext": reminder}}))
PY

python3 -I -c "$PROG" "$(dirname "${BASH_SOURCE[0]}")/lib" \
    || echo "issue-work-complete: the reminder could not run; follow github-issues.md -> Moment 3 by hand." >&2
exit 0
