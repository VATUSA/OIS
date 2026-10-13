#!/bin/bash
# ==============================================================================
# Shared tool-input parsing for the Claude Code hooks.
#
# Every hook used to carry its own copy of this python3 heredoc:
#
#   COMMAND=$(echo "$INPUT" | python3 -c "
#   try:
#       data = json.load(sys.stdin)
#       print(data.get('tool_input', {}).get('command', ''))
#   except:
#       print('')
#   " 2>/dev/null)
#
# The `except: print('')` was the bug. ANY parse fault — python3 absent, shadowed,
# or a malformed payload — printed an empty string and exited 0. An empty command
# matches none of the gates' grep patterns, so every gate fell through to `exit 0`
# = ALLOW. The whole safety net silently stopped protecting while still appearing
# installed, and no gate could tell "parsed, nothing dangerous" from "did not
# parse at all". python3 is not a documented prerequisite on either platform.
#
# Two functions, because the right failure mode differs by hook:
#   * The GATES must fail CLOSED — a gate that cannot read its input must block.
#   * The ADVISORY hooks (doc reminders, rules-on-demand) must fail OPEN — they
#     protect nothing, and blocking a Bash call because an advisory hook could
#     not parse would be worse than the hook not running.
#
# Side-effect-free at source time.
# ==============================================================================

# tool_input_field FIELD
#
# Reads the hook payload on stdin, prints tool_input.FIELD (empty if the key is
# absent — that is a legitimate "parsed, no value", not an error).
#
# Returns 0 on a successful parse, non-zero if the payload could not be parsed at
# all. FIELD is passed as argv, never interpolated into the program text.
tool_input_field() {
    python3 -I -c '
import sys, json
data = json.load(sys.stdin)
value = data.get("tool_input", {}).get(sys.argv[1], "")
sys.stdout.write("" if value is None else str(value))
' "$1" 2>/dev/null
}

# require_tool_input_field FIELD
#
# For GATES. Prints tool_input.FIELD, or blocks with exit 2 if the payload cannot
# be parsed. Call as:  VALUE="$(require_tool_input_field command <<<"$INPUT")" || exit 2
#
# The caller must keep the `|| exit 2`: this function runs in a command
# substitution subshell, so its own `exit 2` ends the subshell, not the gate.
require_tool_input_field() {
    local value status
    value="$(tool_input_field "$1")"
    status=$?

    if (( status != 0 )); then
        echo "BLOCKED: hook could not parse its tool input (python3 missing, or malformed payload)." >&2
        echo "Refusing to allow the command unchecked — a gate that cannot read its input must not pass it." >&2
        return 2
    fi

    printf '%s' "$value"
}

# policy SCRIPT ARGS...
#
# For GATES. Runs `python3 -I SCRIPT ARGS...` with $COMMAND on stdin. Sets POLICY_OUT to its stdout and
# POLICY_WHY to its first stderr line with the `name: ` prefix dropped (the parser's reason for refusing
# the line, so a block says why), and returns its status. A temp file that cannot be made returns 2.
policy() {
    local err status
    err="$(mktemp "${TMPDIR:-/tmp}/hook-policy.XXXXXX")" || { POLICY_OUT=""; POLICY_WHY="no temp file"; return 2; }
    POLICY_OUT="$(printf '%s' "$COMMAND" | python3 -I "$@" 2>"$err")"
    status=$?
    POLICY_WHY="$(sed -n '1s/^[a-z_]*: //p' "$err")"
    rm -f "$err"
    return "$status"
}
