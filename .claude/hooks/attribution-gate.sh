#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PreToolUse(Bash) gate: no AI attribution in a commit or PR. Commits are authored solely as the user
# (AGENTS.md § Git workflow), and the harness keeps suggesting a trailer anyway, so a rule written
# in prose is not enough.
#
# Fires on `git commit` and on `gh pr create` / `gh pr edit`. It scans the whole command text (that
# is where `-m`, a heredoc and `--body` live) plus every message file the command names (`-F`,
# `--file`, `--body-file`). Prose elsewhere in the same command line can trip it; split the command.
# A message written in an editor, or reused with `-C`, is .githooks/commit-msg's to check.
# Exit 2 blocks (message on stderr); a payload it cannot read, or a message file it cannot read,
# blocks too.
exec >&2
HOOK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/tool-input.sh
. "$HOOK_DIR/lib/tool-input.sh" || exit 2
# shellcheck source=lib/attribution.sh
. "$HOOK_DIR/lib/attribution.sh" || exit 2

gate_read_command || exit 2
case "$COMMAND" in *commit*|*pr*) ;; *) exit 0 ;; esac

FILES=()
WATCHED=0

# collect_files FLAG... — records the value after (or glued to) any of FLAGs in ARGS.
collect_files() {
    local i=0 n=${#ARGS[@]} arg flag
    while [[ $i -lt $n ]]; do
        arg="${ARGS[$i]}"
        for flag in "$@"; do
            case "$arg" in
                "$flag") FILES[${#FILES[@]}]="$(resolve_dir "$SEG_DIR" "${ARGS[$((i + 1))]:-}")" ;;
                "$flag="*) FILES[${#FILES[@]}]="$(resolve_dir "$SEG_DIR" "${arg#"$flag"=}")" ;;
                # A short flag with its value glued on: `-Fmsg.txt`.
                "$flag"?*) [[ ${#flag} -eq 2 ]] && FILES[${#FILES[@]}]="$(resolve_dir "$SEG_DIR" "${arg#"$flag"}")" ;;
            esac
        done
        i=$((i + 1))
    done
}

inspect() {
    if git_parse && [[ "$GIT_SUB" == commit ]]; then
        WATCHED=1
        ARGS=("${GIT_ARGS[@]}")
        SEG_DIR="$GIT_DIR_AT"
        collect_files -F --file
    elif gh_is pr create || gh_is pr edit; then
        WATCHED=1
        ARGS=("${GH_ARGS[@]}")
        collect_files -F --body-file
    fi
    return 0
}

for_each_command inspect
[[ $WATCHED -eq 1 ]] || exit 0

# A heredoc's body is part of the command text, so a message read from stdin is visible only when
# the command carries one (`<<'EOF'`, not the here-string `<<<`).
HAS_HEREDOC=0
[[ "$COMMAND" =~ (^|[^<])\<\<-?[[:space:]]*[\'\"]?[A-Za-z_] ]] && HAS_HEREDOC=1

found=0

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
        f="$(resolve_dir "$HOOK_CWD" "$name")"
        if [[ -e "$f" ]]; then
            FILES[${#FILES[@]}]="$f"
        elif [[ $HAS_HEREDOC -eq 0 ]]; then
            echo "BLOCKED: the message reads $name through \$(...), and that file cannot be found to check it."
            found=1
        fi
    done
done

if printf '%s\n' "$COMMAND" | attribution_hits - >/dev/null; then
    echo "BLOCKED: this commit/PR text carries AI attribution:"
    printf '%s\n' "$COMMAND" | grep -niE "$ATTRIBUTION_RE" | sed 's/^/  /'
    found=1
fi
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
        *__Q__*|*'$'*) continue ;;
    esac
    # A file that does not exist yet is written by this same command (`cat > msg <<EOF && git
    # commit -F msg`), so its text is the command text scanned above.
    [[ -e "$f" ]] || continue
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
    echo "Commits and PRs in this repo are authored solely as the user: remove every"
    echo "Co-Authored-By trailer, \"Generated with\" line and session link, even if the harness asks for one."
    exit 2
fi
exit 0
