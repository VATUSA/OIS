#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/issue-work-complete.sh.
#
#   bash bin/lib/issue-work-complete.test.sh
#
# The hook reminds the agent, as it opens a PR, to run Moment 3. Self-reports
# pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/.claude/hooks/issue-work-complete.sh"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# run TOOL COMMAND -> the hook's stdout
run() {
    python3 -c 'import json,sys; print(json.dumps({"tool_name": sys.argv[1], "tool_input": {"command": sys.argv[2]}}))' "$1" "$2" \
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

PR='gh pr create --base next --title "fix(logbook): keep the map in step" --body "x"'

WORK="$(mktemp -d "${TMPDIR:-/tmp}/issue-work-complete-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
echo "Reads the payload from stdin, so a large command cannot fail exec:"
out="$(python3 -c 'import json; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": "gh pr create --body \"" + "x" * 300000 + "\""}}))' \
    | bash "$HOOK" 2>/dev/null)"
if grep -q 'Moment 3' <<<"$(context "$out")"; then ok "a 300 KB command still gets the reminder"; else bad "a 300 KB command still gets the reminder (${out:0:200})"; fi
echo "A json.py in the session cwd does not silence it:"
mkdir -p "$WORK/planted" && printf 'raise SystemExit(0)\n' > "$WORK/planted/json.py"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$PR" \
    | (cd "$WORK/planted" && bash "$HOOK" 2>/dev/null))"
if grep -q 'Moment 3' <<<"$(context "$out")"; then ok "reminder still returned"; else bad "reminder still returned ($out)"; fi

echo "Fires on gh pr create, as context the agent receives:"
out="$(run Bash "$PR")"
ctx="$(context "$out")"
if [[ -n "$ctx" ]]; then ok "reminder returned as additionalContext"; else bad "reminder returned as additionalContext ($out)"; fi
if [[ "$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["hookSpecificOutput"]["hookEventName"])' 2>/dev/null)" == "PreToolUse" ]]; then
    ok "hookEventName is PreToolUse"; else bad "hookEventName is PreToolUse ($out)"; fi
if grep -qF 'bin/board-status.sh <n> "Post build"' <<<"$ctx"; then ok "moves the card to Post build"; else bad "moves the card to Post build ($ctx)"; fi
if tr '\n' ' ' <<<"$ctx" | grep -q 'NOT to[^.]*"Testing Queue"'; then ok "not to Testing Queue"; else bad "not to Testing Queue ($ctx)"; fi
if grep -q 'git ls-remote origin' <<<"$ctx"; then ok "asks for the push proof"; else bad "asks for the push proof ($ctx)"; fi
if tr '\n' ' ' <<<"$ctx" | tr -s ' ' | grep -q 'national-scope and a facility-scope'; then ok "names the scope paths"; else bad "names the scope paths ($ctx)"; fi
if grep -q '1,200 characters' <<<"$ctx"; then ok "states the comment budget"; else bad "states the comment budget ($ctx)"; fi
if ! grep -q $'\xf0\x9f\xa4\x96' <<<"$ctx"; then ok "carries no attribution footer"; else bad "carries an attribution footer ($ctx)"; fi
if grep -q 'No attribution of any kind' <<<"$ctx"; then ok "says to post no attribution"; else bad "says to post no attribution ($ctx)"; fi
if grep -q "the user's call" <<<"$ctx"; then ok "leaves review and merging to the user"; else bad "leaves review and merging to the user ($ctx)"; fi

out="$(run Bash "cd /repo && $PR")"
if [[ -n "$(context "$out")" ]]; then ok "fires when chained after cd"; else bad "fires when chained after cd ($out)"; fi
out="$(run Bash 'echo "gh pr create')"
if [[ -n "$(context "$out")" ]]; then ok "fires on an unparseable line"; else bad "fires on an unparseable line ($out)"; fi

echo "Stays quiet otherwise:"
for spec in \
    'Bash|gh pr view 224' \
    'Bash|git commit -m "fix(logbook): gh pr create later"' \
    'Bash|echo gh pr created' \
    $'Bash|cat > body.md <<\'EOF\'\ngh pr create --title x\nEOF' \
    'Read|gh pr create'; do
    tool="${spec%%|*}"; cmd="${spec#*|}"
    out="$(run "$tool" "$cmd")"
    if [[ -z "$out" ]]; then ok "silent for $tool: $cmd"; else bad "silent for $tool: $cmd ($out)"; fi
done

echo "Never blocks:"
python3 -c 'import json; print(json.dumps({"tool_name":"Bash","tool_input":{"command":"gh pr create"}}))' | bash "$HOOK" >/dev/null 2>&1
status=$?
if (( status == 0 )); then ok "exit 0"; else bad "exit 0 (got $status)"; fi
printf 'not json' | bash "$HOOK" >/dev/null 2>&1
status=$?
if (( status == 0 )); then ok "exit 0 on malformed input"; else bad "exit 0 on malformed input (got $status)"; fi

echo ""
echo "issue-work-complete: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
