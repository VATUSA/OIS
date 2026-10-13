#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/session-context.sh and its SessionStart registration.
#
#   bash bin/lib/session-context.test.sh
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/.claude/hooks/session-context.sh"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# Every run is a session in a worktree of a scratch primary checkout on next with the gates; the primary-checkout
# cases below change it.
WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/session-context-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$WORK"' EXIT
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com GIT_CONFIG_NOSYSTEM=1
P="$WORK/primary"
git init -q -b next "$P" && mkdir -p "$P/.claude/hooks" && : > "$P/.claude/hooks/master-protection-gate.sh"
git -C "$P" add -A && git -C "$P" -c core.hooksPath=/dev/null commit -qm init
git -C "$P" worktree add -q -b feat/1/some-work "$WORK/wt" 2>/dev/null
export CLAUDE_PROJECT_DIR="$WORK/wt"

out="$(bash "$HOOK" </dev/null)"
status=$?
if (( status == 0 )); then ok "exit 0"; else bad "exit 0 (got $status)"; fi
if printf '%s' "$out" | grep -q 'select:TaskCreate,TaskUpdate,TaskList,TaskGet'; then ok "names the Task tools to load"; else bad "names the Task tools to load ($out)"; fi
if printf '%s' "$out" | grep -q 'every task you generate'; then ok "covers self-generated tasks"; else bad "covers self-generated tasks ($out)"; fi
if printf '%s' "$out" | grep -q 'never keep the list in prose'; then ok "forbids a prose fallback"; else bad "forbids a prose fallback ($out)"; fi
if printf '%s' "$out" | grep -q 'CLAUDE_CODE_ENABLE_TODO_TOOLS'; then ok "names the enabling flag"; else bad "names the enabling flag ($out)"; fi
if grep -q "The task list is the operator's view" "$ROOT/CLAUDE.md"; then ok "the CLAUDE.md heading it cites exists"; else bad "the CLAUDE.md heading it cites exists"; fi

flag="$(python3 -c 'import json,sys
print(json.load(open(sys.argv[1])).get("env", {}).get("CLAUDE_CODE_ENABLE_TODO_TOOLS", ""))' "$ROOT/.claude/settings.json")"
if [[ "$flag" == "1" ]]; then ok "settings.json enables the Task tools"; else bad "settings.json enables the Task tools (got '$flag')"; fi

registered="$(python3 -c 'import json,sys
hooks = json.load(open(sys.argv[1])).get("hooks", {}).get("SessionStart", [])
print(any("session-context.sh" in h.get("command", "") for entry in hooks for h in entry.get("hooks", [])))' "$ROOT/.claude/settings.json")"
if [[ "$registered" == "True" ]]; then ok "registered under SessionStart"; else bad "registered under SessionStart"; fi

echo "Build environment through \$CLAUDE_ENV_FILE:"
mkdir -p "$WORK/bin" && printf '#!/bin/bash\nexit 0\n' > "$WORK/bin/sccache" && chmod +x "$WORK/bin/sccache"
: > "$WORK/env"
(unset CARGO_TARGET_DIR RUSTC_WRAPPER; CLAUDE_ENV_FILE="$WORK/env" PATH="$WORK/bin:$PATH" bash "$HOOK" </dev/null >/dev/null)
exports="$(env -i bash -c '. "$1"; printf "%s|%s" "${CARGO_TARGET_DIR:-}" "${RUSTC_WRAPPER:-}"' _ "$WORK/env")"
if [[ "$exports" == "|sccache" ]]; then ok "exports sccache and no CARGO_TARGET_DIR (each worktree builds into its own target/)"; else bad "exports ($exports, want |sccache)"; fi
: > "$WORK/env"
(unset CARGO_TARGET_DIR RUSTC_WRAPPER; CLAUDE_ENV_FILE="$WORK/env" PATH="/usr/bin:/bin" bash "$HOOK" </dev/null >/dev/null)
if [[ ! -s "$WORK/env" ]]; then ok "no sccache on PATH: nothing exported"; else bad "without sccache ($(cat "$WORK/env"))"; fi
: > "$WORK/env"
(RUSTC_WRAPPER='' CLAUDE_ENV_FILE="$WORK/env" PATH="$WORK/bin:$PATH" bash "$HOOK" </dev/null >/dev/null)
if [[ ! -s "$WORK/env" ]]; then ok "values the operator set are kept"; else bad "overrode the operator's values ($(cat "$WORK/env"))"; fi
(unset CLAUDE_ENV_FILE; bash "$HOOK" </dev/null >/dev/null)
if [[ $? -eq 0 ]]; then ok "no \$CLAUDE_ENV_FILE: exit 0"; else bad "no \$CLAUDE_ENV_FILE failed"; fi
out="$(CLAUDE_ENV_FILE="$WORK/missing-dir/env" bash "$HOOK" </dev/null 2>&1)"
if [[ $? -eq 0 && "$out" == *"could not write the build environment"* ]]; then ok "an unwritable env file is said, not fatal"; else bad "unwritable env file ($out)"; fi

echo "The primary checkout every hook runs from:"
# context DIR -> sets CTX (stdout, the agent's context), ERR (stderr) and RC for a session in DIR
context() {
    CTX="$(CLAUDE_PROJECT_DIR="$1" bash "$HOOK" </dev/null 2>"$WORK/err")"; RC=$?; ERR="$(cat "$WORK/err")"
}
context "$WORK/wt"
if [[ $RC -eq 0 && "$CTX" != *WARNING* && -z "$ERR" ]]; then ok "positive control: primary on next with the gates, no warning"; else bad "warned on a healthy primary (rc=$RC: $CTX $ERR)"; fi
git -C "$P" -c core.hooksPath=/dev/null checkout -q -b main
context "$WORK/wt"
if [[ "$CTX" == *"WARNING: the primary checkout $P is on main, not next"* ]]; then ok "a primary off next is a warning in the agent's context"; else bad "no warning for a primary on main ($CTX)"; fi
if [[ "$ERR" == *"is on main, not next"* ]]; then ok "...and on stderr"; else bad "...stderr ($ERR)"; fi
if [[ $RC -eq 0 ]]; then ok "...and the hook still exits 0"; else bad "...exit $RC"; fi
git -C "$P" -c core.hooksPath=/dev/null checkout -q next
rm "$P/.claude/hooks/master-protection-gate.sh"
context "$WORK/wt"
if [[ "$CTX" == *"WARNING: the primary checkout $P lacks .claude/hooks/master-protection-gate.sh"* ]]; then ok "a primary without master-protection-gate.sh is a warning"; else bad "no warning for a missing gate ($CTX)"; fi
context "$P"
if [[ "$CTX" == *"lacks .claude/hooks/master-protection-gate.sh"* ]]; then ok "...also for a session in the primary itself"; else bad "primary session ($CTX)"; fi

echo ""
echo "session-context: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
