#!/bin/bash
# PreToolUse hook: a new migration's number must not already belong to a different migration on any
# origin branch.
#
# sqlx applies two files that share a version and then fails on `_sqlx_migrations`' primary key,
# leaving the database half-migrated (#569). .github/scripts/check-migration-versions.sh catches a
# duplicate inside one tree; this catches the collision with another open branch before it is even
# committed (AGENTS.md -> Conventions & gotchas, "Picking a migration number is contended").
#
# Fires on `git commit`. It checks the migrations already staged plus any that a `git add` earlier
# in the same command line stages: a file it names, and every untracked migration any of its paths
# covers (a directory, a glob, `.`), as git itself resolves the path. It reads local `refs/remotes/origin/*` only, with no fetch, so it
# is as fresh as your last `git fetch`. Your own branch's remote copy is skipped. The line is parsed
# (git_policy.py migrations), so a commit message naming `git commit` is data, while `cd DIR && git
# add ... && git commit` and `git -C DIR commit` are checked in DIR.
#
# Blocking contract: Claude Code blocks a tool call ONLY on `exit 2`, reading stderr. A payload or
# line it cannot read, or a commit in a checkout it cannot tell, blocks too.
#
# Tests: bin/lib/migration-gate.test.sh
exec >&2

HOOK_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh"
if [[ ! -f "$HOOK_LIB" ]]; then
    echo "BLOCKED: cannot locate .claude/hooks/lib/tool-input.sh — refusing to pass the command unchecked."
    exit 2
fi
# shellcheck source=lib/tool-input.sh
. "$HOOK_LIB"

INPUT=$(cat)
COMMAND="$(printf '%s' "$INPUT" | require_tool_input_field command)" || exit 2
case "$COMMAND" in *commit*) ;; *) exit 0 ;; esac

# The session's cwd from the payload, where Claude Code runs the command.
HOOK_CWD="$(printf '%s' "$INPUT" | python3 -I -c '
import json, os, sys
cwd = json.load(sys.stdin).get("cwd")
sys.stdout.write(cwd if isinstance(cwd, str) and os.path.isdir(cwd) else "")' 2>/dev/null)"
if [[ -n "$HOOK_CWD" ]] && ! cd "$HOOK_CWD" 2>/dev/null; then
    echo "BLOCKED: cannot cd to the session's directory $HOOK_CWD to check the migrations it commits."
    exit 2
fi

policy "$(dirname "${BASH_SOURCE[0]}")/lib/git_policy.py" migrations
if [[ $? -ne 0 ]]; then
    echo "BLOCKED: ${POLICY_WHY:-the command could not be parsed}, so a migration it commits cannot be checked."
    exit 2
fi

MIGRATIONS=backend/migrations
CANDIDATES=()
COMMIT_DIRS=()

ADD_PATHS=()

add_candidate() {
    local name="${1##*/}"
    [[ "$name" =~ ^[0-9]+_.*\.sql$ ]] && CANDIDATES[${#CANDIDATES[@]}]="$name"
}

# untracked_added DIR: every untracked migration under backend/migrations that a `git add` on the line stages,
# asked of git itself for each PATH (a directory, a glob, `.`, `backend/*`, '*.sql' or a file), so no
# spelling of the path is missed. git_policy.py gives each PATH relative to the repository's top when it
# knows where the add runs; pathspec magic (`:...`) is passed as written.
untracked_added() {
    local path spec
    for path in "${ADD_PATHS[@]}"; do
        case "$path" in
            :*) spec="$path" ;;
            .) spec=":(top)" ;;  # `:(top).` matches nothing
            *) spec=":(top)$path" ;;
        esac
        git -C "$1" ls-files --others --exclude-standard --full-name -- "$spec" 2>/dev/null
    done | grep -E "^$MIGRATIONS/[0-9]+_[^/]*\.sql$"
}

while IFS= read -r line; do
    case "$line" in
        'commit ?')
            echo "BLOCKED: this line commits in a checkout the gate cannot tell (it changes directory or repository"
            echo "in a way it cannot follow), so its migrations cannot be checked. Run git commit from the checkout,"
            echo "or as \`cd DIR && git commit ...\`."
            exit 2
            ;;
        'commit '*) COMMIT_DIRS[${#COMMIT_DIRS[@]}]="${line#commit }" ;;
        'add '*)
            ADD_PATHS[${#ADD_PATHS[@]}]="${line#add }"
            case "${line#add }" in *"$MIGRATIONS"/*) add_candidate "${line#add }" ;; esac
            ;;
    esac
done <<EOF
$POLICY_OUT
EOF
[[ ${#COMMIT_DIRS[@]} -gt 0 ]] || exit 0

collisions=""
for COMMIT_DIR in "${COMMIT_DIRS[@]}"; do
    candidates=("${CANDIDATES[@]}")
    while IFS= read -r staged; do
        [[ -n "$staged" ]] || continue
        name="${staged##*/}"
        [[ "$name" =~ ^[0-9]+_.*\.sql$ ]] && candidates[${#candidates[@]}]="$name"
    done < <(git -C "$COMMIT_DIR" diff --cached --name-only --diff-filter=AR -- ":/$MIGRATIONS" 2>/dev/null
        # Every untracked migration a `git add` on the line stages.
        [[ ${#ADD_PATHS[@]} -gt 0 ]] && untracked_added "$COMMIT_DIR")
    [[ ${#candidates[@]} -gt 0 ]] || continue

    own="$(git -C "$COMMIT_DIR" symbolic-ref --quiet --short HEAD 2>/dev/null)"

    # Every origin branch's migrations directory, as `<tree sha> <ref>`. Most branches share a few trees,
    # so each distinct tree is listed once.
    trees="$(git -C "$COMMIT_DIR" for-each-ref --format='%(refname):backend/migrations %(refname:lstrip=3)' refs/remotes/origin |
        grep -v ' HEAD$' |
        git -C "$COMMIT_DIR" cat-file --batch-check='%(objectname) %(objecttype) %(rest)' |
        awk -v own="$own" '$2 == "tree" && $3 != own { print $1, $3 }' |
        sort -u -k1,1)"

    while read -r tree ref; do
        [[ -n "$tree" ]] || continue
        while IFS= read -r existing; do
            for candidate in "${candidates[@]}"; do
                [[ "$existing" == "$candidate" ]] && continue
                if [[ $((10#${existing%%_*})) -eq $((10#${candidate%%_*})) ]]; then
                    collisions="$collisions  $candidate collides with $existing on origin/$ref"$'\n'
                fi
            done
        done < <(git -C "$COMMIT_DIR" ls-tree --full-tree --name-only "$tree" 2>/dev/null | grep -E '^[0-9]+_')
    done <<<"$trees"
done

if [[ -n "$collisions" ]]; then
    echo "BLOCKED: a staged migration reuses a version another branch already took:"
    printf '%s' "$collisions" | sort -u
    echo "Renumber yours upward, past the highest number on next AND every open branch (a gap is"
    echo "harmless, a repeat half-migrates the database). See AGENTS.md -> Conventions & gotchas."
    exit 2
fi
exit 0
