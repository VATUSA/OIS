#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/rules-on-demand.sh, its PreToolUse registrations, and
# the path-scoped frontmatter that makes it necessary.
#
#   bash bin/lib/rules-on-demand.test.sh
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/.claude/hooks/rules-on-demand.sh"
LIFECYCLE=".claude/rules/ticket-lifecycle.md .claude/rules/github-issues.md"
GH_FILES=".claude/rules/ticket-lifecycle.md .claude/rules/github-issues.md .claude/rules/anti-ai-slop.md"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/rules-on-demand-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# The arguments settings.json passes to the hook for a tool name (empty when it is not registered).
registered() {
    python3 -c 'import json,re,sys
entries = json.load(open(sys.argv[1])).get("hooks", {}).get("PreToolUse", [])
cmds = [h["command"] for e in entries if re.fullmatch(e.get("matcher", ""), sys.argv[2])
        for h in e.get("hooks", []) if "rules-on-demand.sh" in h.get("command", "")]
# The fail-closed form resolves the hook into $f and ends `bash "$f" ARGS` (settings-hooks.test.sh pins it).
print("\n".join(c.rsplit("bash \"$f\" ", 1)[1] if "bash \"$f\" " in c else "" for c in cmds))' "$ROOT/.claude/settings.json" "$1"
}

# run TOOL COMMAND ARGS... -> the additionalContext the hook returns, or nothing
context() {
    local tool="$1" command="$2"
    shift 2
    python3 -c 'import json,sys; print(json.dumps({"tool_name": sys.argv[1], "tool_input": {"command": sys.argv[2]}}))' "$tool" "$command" \
        | bash "$HOOK" "$@" 2>/dev/null | python3 -c 'import json,sys
raw = sys.stdin.read()
if raw.strip():
    try:
        d = json.loads(raw)["hookSpecificOutput"]
        assert d["hookEventName"] == "PreToolUse", d
        print(d["additionalContext"])
    except Exception:
        print("BADJSON:" + raw)'
}

echo "Registration:"
args="$(registered EnterWorktree)"
if [[ "$args" == "$LIFECYCLE" ]]; then ok "EnterWorktree passes ticket-lifecycle.md and github-issues.md"; else bad "EnterWorktree args (got '$args')"; fi
args="$(registered Bash)"
if [[ "$args" == "$GH_FILES" ]]; then ok "Bash passes the gh rule files"; else bad "Bash args (got '$args')"; fi
for tool in ExitWorktree Read Write Edit mcp__sentry__search_issues; do
    args="$(registered "$tool")"
    if [[ -z "$args" ]]; then ok "$tool is not registered"; else bad "$tool is not registered (got '$args')"; fi
done
post="$(python3 -c 'import json,sys
print(any("rules-on-demand.sh" in h.get("command", "") for e in json.load(open(sys.argv[1])).get("hooks", {}).get("PostToolUse", []) for h in e.get("hooks", [])))' "$ROOT/.claude/settings.json")"
if [[ "$post" == "False" ]]; then ok "not registered under PostToolUse"; else bad "not registered under PostToolUse"; fi

echo "EnterWorktree:"
out="$(context EnterWorktree "" $LIFECYCLE)"
if [[ "$out" == *"read in full now: $LIFECYCLE" ]]; then ok "reminds about both lifecycle files"; else bad "reminds about both lifecycle files ($out)"; fi

echo "Bash fires on real gh issue / project / pr invocations:"
for cmd in 'gh issue view 123 --comments' 'gh project item-add 7 --owner VATUSA --url https://github.com/VATUSA/OIS/issues/5' \
           'gh pr create --base next --title "x" --body "y"' 'cd /tmp && gh issue comment 5 --body-file b.md' \
           'gh -R VATUSA/OIS issue view 5' \
           'GH_PAGER= gh pr view 7' "bash -c 'gh issue list --state all'"; do
    out="$(context Bash "$cmd" $GH_FILES)"
    if [[ "$out" == *"read in full now: $GH_FILES" ]]; then ok "fires: $cmd"; else bad "fires: $cmd ($out)"; fi
done

echo "Bash stays quiet when gh issue/project/pr is not run:"
for cmd in 'git status' 'gh auth status' 'gh api repos/VATUSA/OIS' 'git commit -m "docs: gh issue view 1"' \
           'bash bin/board-status.sh 5 "In build"' \
           $'cat > /tmp/x <<\'EOF\'\ngh issue create\nEOF'; do
    out="$(context Bash "$cmd" $GH_FILES)"
    if [[ -z "$out" ]]; then ok "quiet: ${cmd%%$'\n'*}"; else bad "quiet: ${cmd%%$'\n'*} ($out)"; fi
done

echo "Fails open and never blocks:"
out="$(printf 'not json' | bash "$HOOK" x.md 2>/dev/null)"
if [[ "$out" == *"read in full now: x.md"* ]]; then ok "malformed payload still reminds"; else bad "malformed payload still reminds ($out)"; fi
# Built through stdin, never argv: a 300 KB argument would fail exec in the test itself.
big_payload() { head -c 300000 /dev/zero | tr '\0' x | python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1] + sys.stdin.read() + "\""}}))' "$1"; }
out="$(big_payload 'gh pr create --body "' | bash "$HOOK" $GH_FILES 2>/dev/null)"
if [[ "$out" == *"read in full now: $GH_FILES"* ]]; then ok "300 KB gh command parsed from stdin and fires"; else bad "300 KB gh command parsed from stdin and fires (${out:0:200})"; fi
out="$(big_payload 'echo "' | bash "$HOOK" $GH_FILES 2>/dev/null)"
if [[ -z "$out" ]]; then ok "300 KB non-gh command parsed from stdin and stays quiet"; else bad "300 KB non-gh command parsed from stdin and stays quiet (${out:0:200})"; fi
status=0; printf '{}' | bash "$HOOK" x >/dev/null 2>&1 || status=$?
if (( status == 0 )); then ok "exit 0 (advisory)"; else bad "exit 0 (got $status)"; fi
status=0; printf '{"tool_name":"Bash","tool_input":{"command":"gh pr view"}}' | PATH=/nonexistent /bin/bash "$HOOK" x.md >"$WORK/out" 2>/dev/null || status=$?
if (( status == 0 )) && grep -q 'read in full now: x.md' "$WORK/out"; then ok "no python3: static reminder, exit 0"; else bad "no python3: static reminder, exit 0 (status $status, $(cat "$WORK/out"))"; fi
# A json.py in the session cwd must not replace the standard library and silence the reminder.
mkdir -p "$WORK/planted" && printf 'raise SystemExit(0)\n' > "$WORK/planted/json.py"
printf '{"tool_name":"Bash","tool_input":{"command":"gh pr view"}}' | (cd "$WORK/planted" && bash "$HOOK" x.md) >"$WORK/out" 2>/dev/null
if grep -q 'read in full now: x.md' "$WORK/out"; then ok "a planted json.py in the cwd does not silence it"; else bad "a planted json.py in the cwd does not silence it ($(cat "$WORK/out"))"; fi

echo "Frontmatter:"
# path_scoped FILE: paths: frontmatter, then a heading saying it is not loaded at startup.
path_scoped() {
    head -n 1 "$1" | grep -q '^---$' && sed -n '2p' "$1" | grep -q '^paths:' && grep -q '^# .*(not loaded at startup' "$1"
}
# The files this hook names must be path-scoped, or the reminder points at rules already in context.
for rule in ticket-lifecycle github-issues anti-ai-slop; do
    if path_scoped "$ROOT/.claude/rules/$rule.md"; then ok "$rule.md is path-scoped and says so"; else bad "$rule.md is path-scoped and says so"; fi
done
# Every other file with frontmatter says so too, so the reader knows to read it in full on demand.
for f in "$ROOT"/.claude/rules/*.md; do
    head -n 1 "$f" | grep -q '^---$' || continue
    if path_scoped "$f"; then ok "${f##*/} is path-scoped and says so"; else bad "${f##*/} has frontmatter but no paths:/not-loaded heading"; fi
done
for rule in code-quality test-quality response-style engineering-prose; do
    if head -n 1 "$ROOT/.claude/rules/$rule.md" | grep -q '^---$'; then bad "$rule.md stays always loaded"; else ok "$rule.md stays always loaded"; fi
done

if grep -q 'not loaded at startup' "$ROOT/CLAUDE.md"; then ok "CLAUDE.md carries the read triggers"; else bad "CLAUDE.md carries the read triggers"; fi

echo ""
echo "rules-on-demand: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
