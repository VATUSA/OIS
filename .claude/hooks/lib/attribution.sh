#!/usr/bin/env bash
# The AI-attribution patterns OIS never lets into a commit or PR. Sourced by
# .claude/hooks/attribution-gate.sh (the Claude hook) and .githooks/commit-msg (the git hook), so the
# two cannot drift apart. Commits are authored solely as the user: AGENTS.md § Git workflow.
# Sourced, never run; side-effect-free at source time.

# One extended regex, matched case-insensitively.
ATTRIBUTION_RE='co-authored-by:[[:space:]]*claude([^a-z]|$)|noreply@anthropic\.com|generated (with|by) \[?claude code|claude\.ai/code/session'

# attribution_hits FILE...
#
# Prints every line of FILE(s) that carries attribution, as `file:line:text`. Returns 0 when it found
# any, 1 when the files are clean, and 2 when a file could not be read. Pass `-` to read stdin.
attribution_hits() {
    grep -HniE "$ATTRIBUTION_RE" "$@" 2>/dev/null
}
