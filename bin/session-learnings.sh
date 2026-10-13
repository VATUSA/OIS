#!/bin/bash
# ==============================================================================
# The session learnings file: lessons for CLAUDE.md and .claude/, logged during
# a goal and applied on one branch when it ends (CLAUDE.md -> "Editing CLAUDE.md...").
#
#   bin/session-learnings.sh new
#       Creates /tmp/claude-session-learnings-YYYY-MM-DD-N, N the lowest number
#       free today (sessions run side by side), and prints its path.
#   bin/session-learnings.sh add FILE CONTEXT LEARNING FIX
#       Appends one entry, fenced by a separator line, to a file `new` made.
#
# Exits 2 without writing on a bad argument. Bash 3.2 and BSD/GNU safe.
# Tests: bin/lib/session-learnings.test.sh
# ==============================================================================
set -uo pipefail

DIR="${SESSION_LEARNINGS_DIR:-/tmp}"
PREFIX="claude-session-learnings-"

usage() {
    echo "usage: bin/session-learnings.sh new | add FILE CONTEXT LEARNING FIX" >&2
    exit 2
}

case "${1:-}" in
    new)
        [ "$#" -eq 1 ] || usage
        day="$(date +%Y-%m-%d)"
        n=1
        # noclobber makes the create atomic, so two sessions never share a number.
        while ! ( set -o noclobber; : > "$DIR/$PREFIX$day-$n" ) 2>/dev/null; do
            n=$((n + 1))
            [ "$n" -le 999 ] || { echo "session-learnings: cannot create a file in $DIR" >&2; exit 1; }
        done
        printf '%s\n' "$DIR/$PREFIX$day-$n"
        ;;
    add)
        [ "$#" -eq 5 ] || usage
        file="$2"
        case "${file##*/}" in "$PREFIX"[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]-[0-9]*) ;; *) usage ;; esac
        [ -f "$file" ] || { echo "session-learnings: $file does not exist; create it with: bin/session-learnings.sh new" >&2; exit 2; }
        for field in "$3" "$4" "$5"; do
            [ -n "$field" ] || usage
        done
        {
            printf '==================== %s ====================\n' "$(date -u +%Y-%m-%dT%H:%MZ)"
            printf 'Context: %s\n' "$3"
            printf 'Learning: %s\n' "$4"
            printf 'Fix: %s\n\n' "$5"
        } >> "$file" || { echo "session-learnings: cannot append to $file" >&2; exit 1; }
        echo "logged to $file"
        ;;
    *) usage ;;
esac
