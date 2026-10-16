#!/bin/bash
# ==============================================================================
# Tests that .claude/settings.json registers every gate where it must run.
#
#   bash bin/lib/settings-hooks.test.sh
#
# A gate that is not registered blocks nothing however good its own tests are, and
# deleting one entry from settings.json would otherwise go unnoticed. Exits non-zero
# on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SETTINGS="$ROOT/.claude/settings.json"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# registered TOOL HOOK [EVENT] -> "True" when an EVENT (default PreToolUse) entry whose matcher matches TOOL runs
# HOOK from the main checkout
registered() {
    python3 -c 'import json,re,sys
entries = json.load(open(sys.argv[1])).get("hooks", {}).get(sys.argv[4], [])
print(any(re.fullmatch(e.get("matcher", ""), sys.argv[2])
          and "--git-common-dir" in h.get("command", "") and ("/.claude/hooks/" + sys.argv[3] + "\"") in h.get("command", "")
          for e in entries for h in e.get("hooks", [])))' "$SETTINGS" "$1" "$2" "${3:-PreToolUse}"
}

for hook in master-protection-gate.sh branch-ancestry-gate.sh branch-naming-gate.sh pre-pr-gate.sh test-run-gate.sh \
    update-branch-gate.sh marker-write-gate.sh attribution-gate.sh migration-gate.sh; do
    if [[ "$(registered Bash "$hook")" == "True" ]]; then ok "Bash runs $hook"; else bad "Bash runs $hook"; fi
    if [[ -f "$ROOT/.claude/hooks/$hook" ]]; then ok "$hook exists"; else bad "$hook exists"; fi
done
for tool in Write Edit MultiEdit NotebookEdit; do
    if [[ "$(registered "$tool" marker-write-gate.sh)" == "True" ]]; then ok "$tool runs marker-write-gate.sh"; else bad "$tool runs marker-write-gate.sh"; fi
done
if [[ "$(registered Read marker-write-gate.sh)" == "False" ]]; then ok "control: Read does not run marker-write-gate.sh"; else bad "Read runs marker-write-gate.sh"; fi
for tool in Bash Write Edit MultiEdit NotebookEdit; do
    if [[ "$(registered "$tool" marker-audit.sh PostToolUse)" == "True" ]]; then ok "$tool is audited after by marker-audit.sh"; else bad "$tool is audited after by marker-audit.sh"; fi
done
if [[ "$(registered Bash marker-audit.sh PostToolUseFailure)" == "True" ]]; then ok "a failed Bash call is audited by marker-audit.sh"; else bad "a failed Bash call is audited by marker-audit.sh"; fi
if [[ -f "$ROOT/.claude/hooks/marker-audit.sh" ]]; then ok "marker-audit.sh exists"; else bad "marker-audit.sh exists"; fi
if [[ "$(registered Bash marker-audit.sh PreToolUse)" == "False" ]]; then ok "control: marker-audit.sh is not a PreToolUse hook"; else bad "marker-audit.sh runs before the tool"; fi

# OIS's reminders, and the gate the git policies replaced (its rules live in master-protection-gate.sh now).
for tool in Edit Write MultiEdit; do
    if [[ "$(registered "$tool" client-regen-reminder.sh PostToolUse)" == "True" ]]; then ok "$tool is followed by client-regen-reminder.sh"; else bad "$tool is followed by client-regen-reminder.sh"; fi
done
if [[ -f "$ROOT/.claude/hooks/client-regen-reminder.sh" ]]; then ok "client-regen-reminder.sh exists"; else bad "client-regen-reminder.sh exists"; fi
if [[ "$(registered Bash issue-created-board.sh PostToolUse)" == "True" ]]; then ok "Bash is followed by issue-created-board.sh"; else bad "Bash is followed by issue-created-board.sh"; fi
if [[ "$(registered Bash issue-work-complete.sh)" == "True" ]]; then ok "Bash runs issue-work-complete.sh"; else bad "Bash runs issue-work-complete.sh"; fi
if [[ "$(registered ExitPlanMode issue-plan-approved.sh PostToolUse)" == "True" ]]; then ok "ExitPlanMode is followed by issue-plan-approved.sh"; else bad "ExitPlanMode is followed by issue-plan-approved.sh"; fi
if [[ "$(registered ExitWorktree worktree-teardown.sh)" == "True" ]]; then ok "ExitWorktree runs worktree-teardown.sh"; else bad "ExitWorktree runs worktree-teardown.sh"; fi
if ! grep -q 'git-safety-gate' "$SETTINGS" && [[ ! -e "$ROOT/.claude/hooks/git-safety-gate.sh" ]]; then ok "git-safety-gate.sh is gone and unregistered"; else bad "git-safety-gate.sh is still present or registered"; fi
# Every hook settings.json names exists, and every hook in .claude/hooks is registered.
python3 -c 'import json,re,sys
cmds = [h.get("command", "") for event in json.load(open(sys.argv[1])).get("hooks", {}).values() for e in event for h in e.get("hooks", [])]
print("\n".join(sorted({m for c in cmds for m in re.findall(r"/\.claude/hooks/([^\"/ ]+\.sh)", c)})))' "$SETTINGS" > "${TMPDIR:-/tmp}/settings-hooks.$$"
while IFS= read -r hook; do
    if [[ -f "$ROOT/.claude/hooks/$hook" ]]; then ok "registered $hook exists"; else bad "registered $hook does not exist"; fi
done < "${TMPDIR:-/tmp}/settings-hooks.$$"
for f in "$ROOT"/.claude/hooks/*.sh; do
    if grep -qx "${f##*/}" "${TMPDIR:-/tmp}/settings-hooks.$$"; then ok "${f##*/} is registered"; else bad "${f##*/} is not registered"; fi
done
rm -f "${TMPDIR:-/tmp}/settings-hooks.$$"

# Fail closed. Claude Code runs a hook command and proceeds unless it exits 2, so `bash <missing file>` (exit 127)
# lets every tool call through unchecked. Each entry resolves the hook into $f, exits 2 when the file is missing,
# then runs it with its arguments.
echo "Every hook command fails closed when its file is missing:"
python3 -c 'import json,sys
for event, entries in json.load(open(sys.argv[1])).get("hooks", {}).items():
    for e in entries:
        for h in e.get("hooks", []):
            print(event + "\t" + h.get("command", "").replace("\n", " "))' "$SETTINGS" > "${TMPDIR:-/tmp}/settings-cmds.$$"
FC_RE='^f="\$\(dirname "\$\(git -C "\$\{CLAUDE_PROJECT_DIR:-\.\}" rev-parse --path-format=absolute --git-common-dir\)"\)/\.claude/hooks/[a-z-]+\.sh"; \[ -f "\$f" \] \|\| \{ echo "hook missing: \$f — the primary checkout must be on next with the current hooks" >&2; exit 2; \}; bash "\$f"( \.claude/rules/[a-z-]+\.md)*$'
SH="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/settings-hooks-run.XXXXXX")" && pwd -P)"
git init -q -b next "$SH/primary"
mkdir -p "$SH/primary/.claude/hooks"
COUNT=0
while IFS=$'\t' read -r event command; do
    COUNT=$((COUNT + 1))
    hook="$(grep -oE '/\.claude/hooks/[a-z-]+\.sh' <<<"$command")"; hook="${hook##*/}"
    if [[ "$command" =~ $FC_RE ]]; then ok "$event $hook: the fail-closed form"; else bad "$event $hook is not the fail-closed form: $command"; fi
    rm -f "$SH/primary/.claude/hooks/"*
    out="$(cd "$SH" && CLAUDE_PROJECT_DIR="$SH/primary" sh -c "$command" </dev/null 2>&1)"; rc=$?
    if [[ $rc -eq 2 && "$out" == *"hook missing: $SH/primary/.claude/hooks/$hook"* ]]; then ok "...missing, it exits 2 and names the file"; else bad "$event $hook missing: rc=$rc ($out)"; fi
    printf '#!/bin/bash\necho "ran${*:+ $*} stdin=$(cat)"\nexit 0\n' > "$SH/primary/.claude/hooks/$hook"
    out="$(cd "$SH" && printf 'payload' | CLAUDE_PROJECT_DIR="$SH/primary" sh -c "$command" 2>&1)"; rc=$?
    want="ran$(grep -oE '( \.claude/rules/[a-z-]+\.md)*$' <<<"$command") stdin=payload"
    if [[ $rc -eq 0 && "$out" == "$want" ]]; then ok "...present, it runs with its arguments and the payload"; else bad "$event $hook present: rc=$rc ($out, want $want)"; fi
done < "${TMPDIR:-/tmp}/settings-cmds.$$"
[[ $COUNT -ge 20 ]] && ok "every entry was checked ($COUNT)" || bad "only $COUNT hook commands found"
rm -rf "$SH" "${TMPDIR:-/tmp}/settings-cmds.$$"

echo ""
echo "settings-hooks: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
