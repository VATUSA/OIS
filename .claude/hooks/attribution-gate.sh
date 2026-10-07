#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PreToolUse(Bash) gate: no AI attribution in a commit, PR, issue or comment. Nothing is credited to
# an agent (AGENTS.md § Git workflow, #754), and the harness keeps suggesting a trailer or footer
# anyway, so a rule written in prose is not enough.
#
# Fires on `git commit`; on `gh pr create|edit|comment|review|close|reopen` and
# `gh issue create|comment|edit|close|reopen`; and on every `gh api` call that writes (a method other
# than GET/HEAD, or fields or `--input` with no method), whatever its endpoint, since issue and
# comment bodies, PR review comments and GraphQL mutations all travel that way. It scans the whole
# command text (that is where `-m`, a heredoc, `--body` and `-f body=` live) plus every message file
# the command names (`-F`, `--file`, `--body-file`, `gh api -F key=@file`, `gh api --input`).
# Prose elsewhere in the same command line can trip it; split the command.
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
case "$COMMAND" in *commit*|*gh*) ;; *) exit 0 ;; esac

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

# gh_posts_text — 0 when WORDS is a gh command that writes a commit-like text: a PR or issue body, a
# comment, a review, or the comment `close`/`reopen` can carry. Sets GH_ARGS like gh_is.
gh_posts_text() {
    local pair
    for pair in "pr create" "pr edit" "pr comment" "pr review" "pr close" "pr reopen" \
        "issue create" "issue comment" "issue edit" "issue close" "issue reopen"; do
        gh_is "${pair% *}" "${pair#* }" && return 0
    done
    return 1
}

# api_field VALUE — a `gh api -F key=@file` reads the value from a file (`@-` is stdin). A raw `-f`
# field never does, so it is not passed here.
api_field() {
    case "$1" in
        *=@*) API_FILES[${#API_FILES[@]}]="$(resolve_dir "$SEG_DIR" "${1#*=@}")" ;;
    esac
}

# gh_api_write — 0 when WORDS is a `gh api` call that sends something: an explicit method other than
# GET or HEAD (one it cannot read counts as a write), or, with no method, any field or `--input`,
# which make gh POST. Leaves the files it would read the body from in API_FILES.
gh_api_write() {
    [[ ${#WORDS[@]} -gt 1 && "${WORDS[0]##*/}" == gh ]] || return 1
    local i=1 n=${#WORDS[@]} w next method="" sends=0
    API_FILES=()
    while [[ $i -lt $n ]]; do
        case "${WORDS[$i]}" in
            api) break ;;
            -R|--repo) i=$((i + 2)) ;;
            -*) i=$((i + 1)) ;;
            *) return 1 ;;
        esac
    done
    [[ $i -lt $n ]] || return 1
    i=$((i + 1))
    while [[ $i -lt $n ]]; do
        w="${WORDS[$i]}"
        next="${WORDS[$((i + 1))]:-}"
        case "$w" in
            -X|--method) method="$next"; i=$((i + 2)); continue ;;
            --method=*) method="${w#--method=}" ;;
            -X?*) method="${w#-X}"; method="${method#=}" ;;
            -f|--raw-field) sends=1; i=$((i + 2)); continue ;;
            -f?*|--raw-field=*) sends=1 ;;
            -F|--field) sends=1; api_field "$next"; i=$((i + 2)); continue ;;
            --field=*) sends=1; api_field "${w#--field=}" ;;
            -F?*) sends=1; w="${w#-F}"; api_field "${w#=}" ;;
            --input) sends=1; API_FILES[${#API_FILES[@]}]="$(resolve_dir "$SEG_DIR" "$next")"; i=$((i + 2)); continue ;;
            --input=*) sends=1; API_FILES[${#API_FILES[@]}]="$(resolve_dir "$SEG_DIR" "${w#--input=}")" ;;
        esac
        i=$((i + 1))
    done
    case "$method" in
        '') [[ $sends -eq 1 ]] ;;
        [Gg][Ee][Tt]|[Hh][Ee][Aa][Dd]) return 1 ;;
        *) return 0 ;;
    esac
}

inspect() {
    if git_parse && [[ "$GIT_SUB" == commit ]]; then
        WATCHED=1
        ARGS=("${GIT_ARGS[@]}")
        SEG_DIR="$GIT_DIR_AT"
        collect_files -F --file
    elif gh_posts_text; then
        WATCHED=1
        ARGS=("${GH_ARGS[@]}")
        collect_files -F --body-file
    elif gh_api_write; then
        WATCHED=1
        local f
        for f in "${API_FILES[@]}"; do FILES[${#FILES[@]}]="$f"; done
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

hits="$(printf '%s\n' "$COMMAND" | attribution_hits -)"
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
    echo "Commits, PRs, issues and comments in this repo are authored solely as the user: remove every"
    echo "Co-Authored-By trailer, \"Generated with\" or \"Drafted by\" line and session link, even if the"
    echo "harness asks for one."
    exit 2
fi
exit 0
