#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/issue-plan-approved.sh.
#
#   bash bin/lib/issue-plan-approved.test.sh
#
# The hook reminds the agent, after ExitPlanMode, to post the Moment 2 comment.
# Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/.claude/hooks/issue-plan-approved.sh"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

out="$(printf '{"tool_name":"ExitPlanMode","tool_input":{}}' | bash "$HOOK" 2>/dev/null)"
status=$?
event="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["hookSpecificOutput"]["hookEventName"])' 2>/dev/null)"
ctx="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["hookSpecificOutput"]["additionalContext"])' 2>/dev/null)"

if (( status == 0 )); then ok "exit 0"; else bad "exit 0 (got $status)"; fi
if [[ "$event" == "PostToolUse" ]]; then ok "hookEventName is PostToolUse"; else bad "hookEventName is PostToolUse ($out)"; fi
if printf '%s' "$ctx" | grep -q 'Moment 2'; then ok "names Moment 2"; else bad "names Moment 2 ($ctx)"; fi
if printf '%s' "$ctx" | grep -q '600 characters'; then ok "states the 600-character budget"; else bad "states the 600-character budget ($ctx)"; fi
# OIS credits nothing to an agent (AGENTS.md -> Git workflow): the reminder asks for no footer and carries none.
if ! printf '%s' "$ctx" | grep -q $'\xf0\x9f\xa4\x96'; then ok "carries no attribution footer"; else bad "carries an attribution footer ($ctx)"; fi
if printf '%s' "$ctx" | grep -q 'No attribution of any kind'; then ok "says to post no attribution"; else bad "says to post no attribution ($ctx)"; fi
if printf '%s' "$ctx" | grep -qF 'bin/board-status.sh <n> "In build"'; then ok "moves the card to In build"; else bad "moves the card to In build ($ctx)"; fi

printf 'not json' | bash "$HOOK" >/dev/null 2>&1
status=$?
if (( status == 0 )); then ok "exit 0 on malformed input"; else bad "exit 0 on malformed input (got $status)"; fi

# The hook runs python from stdin (`python3 -I -`), where a plain `python3 -` would import the cwd's json.py.
PLANTED="$(mktemp -d "${TMPDIR:-/tmp}/plan-approved-test.XXXXXX")"
trap 'rm -rf "$PLANTED"' EXIT
printf 'raise SystemExit(0)\n' > "$PLANTED/json.py"
out="$(printf '{"tool_name":"ExitPlanMode","tool_input":{}}' | (cd "$PLANTED" && bash "$HOOK" 2>/dev/null))"
if printf '%s' "$out" | grep -q 'Moment 2'; then ok "a planted json.py in the cwd does not silence it"; else bad "planted json.py silenced the reminder ($out)"; fi

echo ""
echo "issue-plan-approved: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
