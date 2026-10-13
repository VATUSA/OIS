#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/issue-created-board.sh.
#
#   bash bin/lib/issue-created-board.test.sh
#
# The hook reminds the agent, after `gh issue create`, to put the issue on the
# board in Triaging, with the number gh printed filled in. Self-reports
# pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/.claude/hooks/issue-created-board.sh"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# run TOOL COMMAND [STDOUT] -> the hook's stdout
run() {
    python3 -c 'import json,sys; print(json.dumps({"tool_name": sys.argv[1], "tool_input": {"command": sys.argv[2]},
        "tool_response": {"stdout": sys.argv[3]}}))' "$1" "$2" "${3:-}" \
        | bash "$HOOK" 2>/dev/null
}

# context OUTPUT -> the additionalContext string, or nothing
context() {
    printf '%s' "$1" | python3 -c 'import json,sys
try:
    print(json.load(sys.stdin)["hookSpecificOutput"]["additionalContext"])
except Exception:
    pass'
}

CREATE='gh issue create --repo VATUSA/OIS --title "Logbook map drops the last leg" --body-file /tmp/b.md'

echo "Fires on gh issue create, as context the agent receives:"
out="$(run Bash "$CREATE" "https://github.com/VATUSA/OIS/issues/231")"
ctx="$(context "$out")"
if [[ -n "$ctx" ]]; then ok "reminder returned as additionalContext"; else bad "reminder returned as additionalContext ($out)"; fi
if [[ "$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["hookSpecificOutput"]["hookEventName"])' 2>/dev/null)" == "PostToolUse" ]]; then
    ok "hookEventName is PostToolUse"; else bad "hookEventName is PostToolUse ($out)"; fi
if grep -q 'issues/231$' <<<"$ctx"; then ok "fills the item-add URL with the issue number"; else bad "fills the item-add URL with the issue number ($ctx)"; fi
if grep -qF 'bash bin/board-status.sh 231 "Triaging"' <<<"$ctx"; then ok "moves it to Triaging with board-status.sh"; else bad "moves it to Triaging with board-status.sh ($ctx)"; fi
if grep -qF 'gh project item-add 7 --owner VATUSA' <<<"$ctx"; then ok "adds it to project 7"; else bad "adds it to project 7 ($ctx)"; fi
if ! grep -q 'item-list [0-9]' <<<"$ctx"; then ok "never asks for the board listing"; else bad "asks for gh project item-list ($ctx)"; fi

out="$(run Bash "$CREATE" "no url printed")"
if grep -q 'issues/<n>$' <<<"$(context "$out")"; then ok "falls back to <n> when gh printed no URL"; else bad "falls back to <n> ($out)"; fi
out="$(run Bash "cd /repo && $CREATE")"
if [[ -n "$(context "$out")" ]]; then ok "fires when chained after cd"; else bad "fires when chained after cd ($out)"; fi
out="$(run Bash 'echo "gh issue create')"
if [[ -n "$(context "$out")" ]]; then ok "fires on an unparseable line"; else bad "fires on an unparseable line ($out)"; fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/issue-created-board-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
echo "Reads the payload from stdin, so a large response cannot fail exec:"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]},
    "tool_response": {"stdout": "x" * 300000 + "\nhttps://github.com/VATUSA/OIS/issues/232"}}))' "$CREATE" \
    | bash "$HOOK" 2>/dev/null)"
if grep -q 'issues/232$' <<<"$(context "$out")"; then ok "a 300 KB tool response still gets the reminder"; else bad "a 300 KB tool response still gets the reminder (${out:0:200})"; fi
echo "A json.py in the session cwd does not silence it:"
mkdir -p "$WORK/planted" && printf 'raise SystemExit(0)\n' > "$WORK/planted/json.py"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]},
    "tool_response": {"stdout": "https://github.com/VATUSA/OIS/issues/233"}}))' "$CREATE" \
    | (cd "$WORK/planted" && bash "$HOOK" 2>/dev/null))"
if grep -q 'issues/233$' <<<"$(context "$out")"; then ok "reminder still returned"; else bad "reminder still returned ($out)"; fi

echo "Stays quiet otherwise:"
for spec in \
    'Bash|gh issue view 231' \
    'Bash|git commit -m "docs: gh issue create then gh project item-add"' \
    $'Bash|cat > notes.md <<\'EOF\'\ngh issue create --title x\nEOF' \
    'Read|gh issue create'; do
    tool="${spec%%|*}"; cmd="${spec#*|}"
    out="$(run "$tool" "$cmd")"
    if [[ -z "$out" ]]; then ok "silent for $tool: $cmd"; else bad "silent for $tool: $cmd ($out)"; fi
done

echo "Never blocks:"
printf 'not json' | bash "$HOOK" >/dev/null 2>&1
status=$?
if (( status == 0 )); then ok "exit 0 on malformed input"; else bad "exit 0 on malformed input (got $status)"; fi

echo ""
echo "issue-created-board: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
