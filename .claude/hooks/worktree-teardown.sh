#!/bin/bash
# PreToolUse hook on ExitWorktree: tear the worktree down when action="remove".
#
# This exists as a script rather than an inline one-liner because the one-liner it
# replaces ended in `|| true`:
#
#   [ "$ACTION" = "remove" ] && bash .../teardown-worktree.sh || true
#
# The `|| true` was load-bearing for the action!="remove" case, but it also
# swallowed teardown's exit 1. So when teardown BLOCKED on a dirty worktree, the
# hook still returned 0, ExitWorktree removed the directory anyway, and the
# database was left orphaned — teardown exits before dropping it.
# The guard protecting uncommitted work was decorative.
#
# Blocking contract: Claude Code blocks a tool call only on `exit 2`, and reads
# stderr when it does. Exit 1 is a non-blocking error and the tool proceeds.
exec >&2

HOOK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

HOOK_LIB="$HOOK_DIR/lib/tool-input.sh"
if [[ ! -f "$HOOK_LIB" ]]; then
    echo "BLOCKED: cannot locate .claude/hooks/lib/tool-input.sh — refusing to remove the worktree unchecked."
    exit 2
fi
# shellcheck source=lib/tool-input.sh
. "$HOOK_LIB"

INPUT=$(cat)

# Fails closed. A parse fault used to print '' and exit 0, which is NOT "remove",
# so the hook returned 0 and ExitWorktree removed the worktree with teardown never
# running at all — the exact data loss this hook exists to prevent.
ACTION="$(printf '%s' "$INPUT" | require_tool_input_field action)" || exit 2

# Anything other than a removal is none of our business.
if [[ "$ACTION" != "remove" ]]; then
    exit 0
fi

TEARDOWN="$HOOK_DIR/../../bin/teardown-worktree.sh"
if [[ ! -f "$TEARDOWN" ]]; then
    echo "worktree-teardown: cannot locate bin/teardown-worktree.sh — refusing to remove the worktree unchecked."
    exit 2
fi

# The worktree name is derived from the cwd, so the cwd MUST be verified first.
#
# This previously called teardown with no argument and relied on an unverified
# comment ("the session's cwd IS the worktree at this point"). When it wasn't —
# cwd being the main repo, say — teardown took the name from basename "$PWD",
# found no worktree at .claude/worktrees/<that>, took its "drop the database
# only" branch (a no-op), and exited 0. The hook passed, ExitWorktree
# removed the real worktree, and its uncommitted work was gone with the database
# orphaned. Silently, because every step "succeeded".
#
# HOOK_DIR is used rather than git rev-parse: this file always lives in the primary
# checkout (settings.json resolves it via --git-common-dir), and it cannot be fooled
# by a stray GIT_DIR in the environment.
# Both sides resolved with `pwd -P`. Comparing a logical path to a physical one
# would diverge over any symlinked component (/tmp -> /private/tmp on macOS) and
# refuse a legitimate teardown.
MAIN_ROOT="$(cd "$HOOK_DIR/../.." && pwd -P)"
CWD="$(pwd -P)"

if [[ "$(dirname "$CWD")" != "$MAIN_ROOT/.claude/worktrees" ]]; then
    echo "BLOCKED: ExitWorktree was called from $CWD, which is not a worktree under"
    echo "$MAIN_ROOT/.claude/worktrees."
    echo ""
    echo "Refusing to run teardown: it derives the worktree name from the working directory, so"
    echo "from here it would check the wrong worktree, report success, and let the real"
    echo "one be removed with its database orphaned."
    exit 2
fi

# Registration with THIS main repo, not merely "is a git repository". A standalone
# repo sitting at .claude/worktrees/<name> satisfies `rev-parse --show-toplevel
# == $CWD`, and teardown would then resolve its root to that repo rather than to
# the real one, find no worktree under it, drop nothing, and exit 0 — so the hook
# would pass and ExitWorktree would delete it with its uncommitted work.
#
# Paths are normalized through `pwd -P` on both sides: git prints the path it was
# registered with, which may differ from the resolved one over any symlinked
# component (/tmp -> /private/tmp on macOS).
REGISTERED=false
WORKTREE_PATHS="$(git -C "$MAIN_ROOT" worktree list --porcelain 2>/dev/null | sed -n 's/^worktree //p')"
while IFS= read -r candidate; do
    [ -n "$candidate" ] || continue
    resolved="$(cd "$candidate" 2>/dev/null && pwd -P)" || continue
    if [ "$resolved" = "$CWD" ]; then
        REGISTERED=true
        break
    fi
done <<EOF
$WORKTREE_PATHS
EOF

if [[ "$REGISTERED" != true ]]; then
    echo "BLOCKED: $CWD sits under .claude/worktrees but is not a registered worktree of"
    echo "$MAIN_ROOT."
    echo ""
    echo "It may be a leftover directory, or a separate repository that happens to sit"
    echo "there. Either way teardown would work off the wrong root and skip its"
    echo "uncommitted-work check. Resolve the state by hand."
    exit 2
fi

# Pass the worktree name explicitly rather than letting teardown re-derive it from a cwd
# that may differ from the one verified above.
# WORKTREE_TEARDOWN_FROM_HOOK lets teardown override this session's own worktree lock,
# which it refuses to do for any other caller.
if WORKTREE_TEARDOWN_FROM_HOOK=1 bash "$TEARDOWN" "$(basename "$CWD")"; then
    exit 0
fi

echo ""
echo "BLOCKED: teardown did not complete, and the worktree directory is still in place."
echo "Resolve the issue above and retry: every teardown step is idempotent. The full"
echo "output is in .claude/worktree-teardown.log in the primary checkout."
exit 2
