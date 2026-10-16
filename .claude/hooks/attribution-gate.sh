#!/bin/bash
# PreToolUse hook: no AI attribution in a commit, PR, issue or comment. Nothing is credited to an
# agent (AGENTS.md -> Git workflow, #754), and the harness keeps suggesting a trailer or footer
# anyway, so a rule written in prose is not enough.
#
# Fires on `git commit`; on `gh pr create|new|edit|comment|review|close|reopen|merge` and
# `gh issue create|new|comment|edit|close|reopen`; and on every `gh api` call that writes (a method
# other than GET/HEAD, or fields or `--input` with no method), whatever its endpoint, since issue and
# comment bodies, PR review comments and GraphQL mutations all travel that way. The line is parsed
# (lib/attribution_targets.py over lib/command_match.py, as every gate here is), so a heredoc or quoted
# argument that only names one of them is data. It scans the whole command text (that is where `-m`, a
# heredoc, `--body` and `-f body=` live), each word on its own at every depth (so a body's last line
# ends at its closing quote, inside `bash -c` too), and every message file the command names (`-F`,
# `--file`, `--body-file`, `gh api -F key=@file`, `gh api --input`).
# Prose elsewhere in the same command line can trip it; split the command, or quote the form in a
# code span (backticks), which is read as a quotation.
# A message written in an editor, or reused with `-C`, is .githooks/commit-msg's to check.
#
# Blocking contract: Claude Code blocks a tool call ONLY on `exit 2`, reading stderr; `exec >&2` keeps
# every message on that stream. It fails closed on a body it cannot read: an unparseable payload or
# line, stdin without a heredoc, a body file that does not exist and that no redirect in the command
# writes, a body from a `$(...)` other than `$(cat f)` / `$(< f)` / a heredoc, a relative body file on
# a line that changes directory, and a `gh api -F key=@path` whose path is a variable. A body held in
# a shell variable it cannot see.
#
# Tests: bin/lib/attribution-gate.test.sh
exec >&2

HOOK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for lib in tool-input.sh attribution.sh; do
    if [[ ! -f "$HOOK_DIR/lib/$lib" ]]; then
        echo "BLOCKED: cannot locate .claude/hooks/lib/$lib — refusing to pass the command unchecked."
        exit 2
    fi
done
# shellcheck source=lib/tool-input.sh
. "$HOOK_DIR/lib/tool-input.sh"
# shellcheck source=lib/attribution.sh
. "$HOOK_DIR/lib/attribution.sh"

INPUT=$(cat)

# Fails closed: a gate that cannot read its input must not pass it.
COMMAND="$(printf '%s' "$INPUT" | require_tool_input_field command)" || exit 2
case "$COMMAND" in *commit*|*gh*) ;; *) exit 0 ;; esac

# Relative paths resolve against the payload's cwd (the session's), not this process's.
HOOK_CWD="$(printf '%s' "$INPUT" | python3 -I -c '
import json, os, sys
cwd = json.load(sys.stdin).get("cwd")
sys.stdout.write(cwd if isinstance(cwd, str) and os.path.isdir(cwd) else "")' 2>/dev/null)"
[[ -n "$HOOK_CWD" ]] || HOOK_CWD="$PWD"

# resolve PATH — PATH made absolute against the payload's cwd.
resolve() {
    case "$1" in
        \~) printf '%s' "$HOME" ;;
        \~/*) printf '%s' "$HOME/${1#\~/}" ;;
        /*) printf '%s' "$1" ;;
        *) printf '%s' "$HOOK_CWD/$1" ;;
    esac
}

RECORDS="$(mktemp "${TMPDIR:-/tmp}/attribution-gate.XXXXXX")" || {
    echo "BLOCKED: the attribution check could not make a temp file."
    exit 2
}
trap 'rm -f "$RECORDS"' EXIT
WHY="$(printf '%s' "$COMMAND" | python3 -I "$HOOK_DIR/lib/attribution_targets.py" "$HOOK_CWD" 2>&1 >"$RECORDS")"
if [[ $? -ne 0 ]]; then
    echo "BLOCKED: ${WHY#attribution_targets: }, so its commit, PR, issue or comment text cannot be checked."
    exit 2
fi

WATCHED=0
API_SEEN=0
FILES=()
WORDS_TEXT=""
found=0
while IFS= read -r -d '' record; do
    case "$record" in
        W) WATCHED=1 ;;
        A) API_SEEN=1 ;;
        F*) FILES[${#FILES[@]}]="${record#F}" ;;
        Q*) WORDS_TEXT="$WORDS_TEXT${record#Q}"$'\n' ;;
        X*) echo "BLOCKED: ${record#X}."; found=1 ;;
    esac
done <"$RECORDS"
[[ $WATCHED -eq 1 ]] || exit 0

# A heredoc's body is part of the command text, so a message read from stdin is visible only when
# the command carries one (`<<'EOF'`, not the here-string `<<<`).
HAS_HEREDOC=0
[[ "$COMMAND" =~ (^|[^<])\<\<-?[[:space:]]*[\'\"]?[A-Za-z_] ]] && HAS_HEREDOC=1

# Message text that arrives through a substitution: `--body "$(cat body.md)"`, `-m "$(< msg)"`.
# Read the file it names; `$(cat <<'EOF' ...)` is a heredoc, already in the command text.
SUBST_RE='\$\([[:space:]]*(cat[[:space:]]+|<[[:space:]]*)([^]()`$<>|;&[:cntrl:]]+)\)'
rest="$COMMAND"
while [[ "$rest" =~ $SUBST_RE ]]; do
    rest="${rest#*"${BASH_REMATCH[0]}"}"
    read -r -a names <<<"${BASH_REMATCH[2]}"
    for name in "${names[@]}"; do
        name="${name//[\'\"]/}"
        case "$name" in -*|'') continue ;; esac
        f="$(resolve "$name")"
        if [[ -e "$f" ]]; then
            FILES[${#FILES[@]}]="$f"
        elif [[ $HAS_HEREDOC -eq 0 ]]; then
            echo "BLOCKED: the message reads $name through \$(...), and that file cannot be found to check it."
            found=1
        fi
    done
done

# A body from any other substitution (`$(head f)`, `$(cat f | sed ..)`, `$(git log ..)`) is text this
# gate cannot see, so a body flag fed one blocks.
BODY_SUBST_RE='(^|[[:space:]])((--body|-b|--message|-m|--comment|-c)(=|[[:space:]]*)|(-f|-F|--field|--raw-field)(=|[[:space:]]*)["'"'"']?body=)["'"'"']?\$\('
rest="$COMMAND"
while [[ "$rest" =~ $BODY_SUBST_RE ]]; do
    rest="${rest#*"${BASH_REMATCH[0]}"}"
    sub="\$($rest"
    [[ "$sub" =~ ^\$\([[:space:]]*cat[[:space:]]+\<\< || "$sub" =~ ^$SUBST_RE ]] && continue
    echo "BLOCKED: a body comes from a \$(...) this gate cannot read: ${sub:0:60}"
    echo "  Write the body to a file and pass it with --body-file / -F / -F body=@file."
    found=1
done

# `gh api -F "body=@$F"`: the quoted value hides the path from a reader of the words, so find it in the raw text.
if [[ $API_SEEN -eq 1 ]]; then
    FIELD_RE="(^|[[:space:]])(-F|--field)(=|[[:space:]]*)(\"[^\"]*\"|'[^']*')"
    rest="$COMMAND"
    while [[ "$rest" =~ $FIELD_RE ]]; do
        rest="${rest#*"${BASH_REMATCH[0]}"}"
        v="${BASH_REMATCH[4]}"
        v="${v:1:${#v}-2}"
        case "$v" in *=@*) path="${v#*=@}" ;; *) continue ;; esac
        [[ "$path" =~ ^[A-Za-z0-9_./:@%+,~^-]*$ ]] && continue
        echo "BLOCKED: gh api reads a field from @$path, a path this gate cannot resolve to check it."
        echo "  Pass a plain path: -F body=@body.md"
        found=1
    done
fi

# written_here PATH — 0 when COMMAND redirects (`>`, `>>`) or tees into a file named like PATH.
written_here() {
    local esc re
    esc="$(printf '%s' "${1##*/}" | sed 's/[][\.*^$+?(){}|]/\\&/g')"
    re="(>|tee([[:space:]]+-[A-Za-z-]+)*)[[:space:]]*[\"']?([^[:space:]\"'<>|;&]*/)?${esc}([\"'[:space:]);&|]|\$)"
    [[ "$COMMAND" =~ $re ]]
}

# The command text, then every word on its own: `--body "...footer" --title t` puts the footer's line
# ahead of more arguments in the command text, but alone in its word.
hits="$(printf '%s\n%s' "$COMMAND" "$WORDS_TEXT" | attribution_hits -)"
case $? in
    0)
        echo "BLOCKED: this commit, PR, issue or comment text carries AI attribution:"
        printf '%s\n' "$hits" | sed 's/^(standard input):/  line /'
        found=1
        ;;
    1) ;;
    *)
        echo "BLOCKED: the attribution check could not run on this command's text."
        found=1
        ;;
esac
for f in "${FILES[@]}"; do
    case "$f" in
        */-|-|/dev/stdin|/dev/fd/0)
            # stdin: a heredoc's body is already scanned as command text; a pipe or a redirect is not.
            if [[ $HAS_HEREDOC -eq 0 ]]; then
                echo "BLOCKED: the message comes from stdin (a pipe or a redirect), which this gate cannot read."
                echo "  Write it to a file and pass the path: --body-file body.md / git commit -F msg.txt"
                found=1
            fi
            continue
            ;;
        *'$'*|*'`'*) continue ;;  # a path the shell builds: the raw-text checks above judge it
    esac
    # A file that does not exist yet passes only when a redirect in this same command writes it
    # (`cat > msg <<EOF && git commit -F msg`), so its text is the command text scanned above.
    # One made by `cp` or a tool the gate cannot read blocks.
    if [[ ! -e "$f" ]]; then
        written_here "$f" && continue
        echo "BLOCKED: message file $f does not exist, and no redirect in this command writes it."
        found=1
        continue
    fi
    attribution_hits "$f" >/dev/null
    case $? in
        0)
            echo "BLOCKED: message file $f carries AI attribution:"
            attribution_hits "$f" | sed 's/^/  /'
            found=1
            ;;
        2)
            echo "BLOCKED: cannot read message file $f to check it for attribution."
            found=1
            ;;
    esac
done
if [[ $found -eq 1 ]]; then
    echo "Commits, PRs, issues and comments in this repo are authored solely as the user: remove every"
    echo "Co-Authored-By trailer, \"Generated with\" or \"Drafted by\" line and session link, even if the"
    echo "harness asks for one. To quote a form in prose, put it in a code span (backticks)."
    exit 2
fi
exit 0
