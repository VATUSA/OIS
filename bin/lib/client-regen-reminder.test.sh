#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/client-regen-reminder.sh.
#
#   bash bin/lib/client-regen-reminder.test.sh
#
# The hook reminds the agent, after an edit that can move the API contract, to
# regenerate the typed client (AGENTS.md -> "The API contract → typed client").
# Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/.claude/hooks/client-regen-reminder.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/client-regen-reminder-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# run TOOL FILE KEY TEXT -> the hook's stdout for an edit tool's payload
run() {
    python3 -c 'import json,sys; print(json.dumps({"hook_event_name": "PostToolUse", "tool_name": sys.argv[1],
        "tool_input": {"file_path": sys.argv[2], sys.argv[3]: sys.argv[4]}}))' "$1" "$2" "$3" "$4" \
        | bash "$HOOK" 2>/dev/null
}

# context OUTPUT -> the additionalContext string, or nothing
context() {
    printf '%s' "$1" | python3 -c 'import json,sys
try:
    d = json.load(sys.stdin)["hookSpecificOutput"]
    assert d["hookEventName"] == "PostToolUse"
    print(d["additionalContext"])
except Exception:
    pass'
}

B="$WORK/backend/src"
mkdir -p "$B/models" "$B/handlers" "$B/repos" "$WORK/web/src/lib"
printf '#[derive(Serialize, ToSchema)]\npub struct Tmi {\n    pub id: i64,\n}\n' > "$B/models/tmi.rs"
printf 'pub struct Row {\n    pub id: i64,\n}\n' > "$B/models/row.rs"

echo "Fires on an edit that can move the contract:"
for spec in "Edit|$B/router.rs|new_string|x" "Edit|$B/openapi.rs|new_string|x" \
    "Edit|$B/handlers/flow.rs|new_string|#[derive(Serialize, ToSchema)]" \
    "Write|$B/handlers/x.rs|content|#[utoipa::path(get)]" \
    "Edit|$B/handlers/flow.rs|old_string|#[derive(ToSchema)]" \
    "Edit|$B/models/tmi.rs|new_string|    /// The TMI's id." \
    "Edit|backend/src/router.rs|new_string|x"; do
    IFS='|' read -r tool file key text <<<"$spec"
    ctx="$(context "$(run "$tool" "$file" "$key" "$text")")"
    if [[ "$ctx" == *"pnpm --filter @ois/api-client codegen"* ]]; then ok "fires: $tool ${file#"$WORK"/} ($key)"; else bad "fires: $tool $file ($ctx)"; fi
done
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "MultiEdit", "tool_input": {"file_path": sys.argv[1],
    "edits": [{"old_string": "a", "new_string": "b"}, {"old_string": "c", "new_string": "#[derive(ToSchema)]"}]}}))' "$B/handlers/y.rs" \
    | bash "$HOOK" 2>/dev/null)"
if [[ "$(context "$out")" == *"ToSchema/utoipa"* ]]; then ok "fires: a MultiEdit whose second edit adds a ToSchema derive"; else bad "MultiEdit ($out)"; fi
ctx="$(context "$(run Edit "$B/router.rs" new_string x)")"
if [[ "$ctx" == *"router.rs registers routes"* && "$ctx" == *"openapi.rs path"* ]]; then ok "says why, and that a route needs its openapi.rs path"; else bad "reason ($ctx)"; fi

echo "Stays quiet otherwise:"
for spec in "Edit|$B/repos/flow.rs|new_string|let x = 1;" "Edit|$WORK/web/src/lib/flow.ts|new_string|ToSchema" \
    "Edit|$B/models/row.rs|new_string|    pub name: String," "Read|$B/repos/flow.rs|new_string|x"; do
    IFS='|' read -r tool file key text <<<"$spec"
    out="$(run "$tool" "$file" "$key" "$text")"
    if [[ -z "$out" ]]; then ok "quiet: $tool ${file#"$WORK"/}"; else bad "quiet: $tool $file ($out)"; fi
done

echo "Never blocks:"
for payload in "" "this is not json" '{"tool_name":"Edit"}' '{"tool_name":"Edit","tool_input":{"file_path":42}}'; do
    out="$(printf '%s' "$payload" | bash "$HOOK" 2>/dev/null)"
    status=$?
    if [[ $status -eq 0 && -z "$out" ]]; then ok "exit 0 and quiet on '${payload:-<empty>}'"; else bad "payload '${payload:-<empty>}' (exit $status, $out)"; fi
done
mkdir -p "$WORK/planted" && printf 'raise SystemExit(0)\n' > "$WORK/planted/json.py"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Edit", "tool_input": {"file_path": sys.argv[1], "new_string": "x"}}))' "$B/router.rs" \
    | (cd "$WORK/planted" && bash "$HOOK" 2>/dev/null))"
if [[ "$(context "$out")" == *codegen* ]]; then ok "a planted json.py in the cwd does not silence it"; else bad "planted json.py silenced it ($out)"; fi

echo ""
echo "client-regen-reminder: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
