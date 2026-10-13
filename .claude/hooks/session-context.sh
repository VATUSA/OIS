#!/bin/bash
# SessionStart: reminders the agent needs before its first piece of work, and the build environment every
# later Bash command in the session runs with.
# SessionStart stdout is added to the agent's context. Tests: bin/lib/session-context.test.sh.
#
# Build environment: RUSTC_WRAPPER=sccache when sccache is on PATH. Claude Code applies the `export` lines a
# SessionStart hook appends to $CLAUDE_ENV_FILE to every later Bash command. A value the operator already set
# is kept, and a missing sccache or env file is never an error. No CARGO_TARGET_DIR: each worktree builds into
# its own target/, seeded by bin/setup-worktree.sh (bin/lib/portable.sh export_build_env says why).
echo "REMINDER: Keep the visible task list current (CLAUDE.md -> \"The task list is the operator's view\"). Load the Task tools now if deferred (ToolSearch select:TaskCreate,TaskUpdate,TaskList,TaskGet); if ToolSearch finds none, the session lacks CLAUDE_CODE_ENABLE_TODO_TOOLS=1 (.claude/settings.json env): tell the operator and never keep the list in prose. Every operator task and every task you generate goes on it immediately."

# Every hook runs from the primary checkout's .claude/hooks (settings.json resolves it through the git common
# dir), so a primary checkout off next, or on a commit without the gates, runs old gates or none.
COMMON="$(git -C "${CLAUDE_PROJECT_DIR:-.}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
if [[ -n "$COMMON" ]]; then
    PRIMARY="$(dirname "$COMMON")"
    PRIMARY_BRANCH="$(git -C "$PRIMARY" symbolic-ref --quiet --short HEAD 2>/dev/null)"
    PROBLEM=""
    if [[ "$PRIMARY_BRANCH" != "next" ]]; then
        PROBLEM="is on ${PRIMARY_BRANCH:-a detached HEAD}, not next"
    fi
    if [[ ! -f "$PRIMARY/.claude/hooks/master-protection-gate.sh" ]]; then
        PROBLEM="${PROBLEM:+$PROBLEM and }lacks .claude/hooks/master-protection-gate.sh"
    fi
    if [[ -n "$PROBLEM" ]]; then
        WARNING="WARNING: the primary checkout ${PRIMARY} ${PROBLEM}. Every Claude Code hook runs from there, so the gates are stale or missing (a missing hook blocks every tool call it guards). Tell the operator: the primary checkout must be on next with the current hooks (git -C ${PRIMARY} switch next && git -C ${PRIMARY} pull --ff-only origin next)."
        echo "$WARNING"
        echo "$WARNING" >&2
    fi
fi

if [[ -n "${CLAUDE_ENV_FILE:-}" ]]; then
    {
        if [[ -z "${RUSTC_WRAPPER+set}" ]] && command -v sccache >/dev/null 2>&1; then
            printf 'export RUSTC_WRAPPER=sccache\n'
        fi
    } >>"$CLAUDE_ENV_FILE" 2>/dev/null \
        || echo "REMINDER: could not write the build environment to \$CLAUDE_ENV_FILE; export RUSTC_WRAPPER=sccache before building when sccache is installed."
fi
echo "BUILD: Cargo builds into this checkout's own target/ (bash bin/setup-worktree.sh seeds a worktree's from the primary checkout's)$(command -v sccache >/dev/null 2>&1 && echo ', through sccache'). Run the full gate once per commit (just test); a targeted cargo nextest or vitest run is always fine."
exit 0
