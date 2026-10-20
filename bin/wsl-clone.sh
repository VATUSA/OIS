#!/bin/bash
set -uo pipefail

# ==============================================================================
# The WSL gate clone, for a Windows host
# ==============================================================================
# `just test` cannot stamp a tests marker on a Windows checkout, so a Windows host gates,
# pushes and opens the PR from a Linux clone of the branch inside WSL. This script creates,
# gates, pushes from and removes that clone, so none is made by hand or left behind.
#
# Run inside WSL, from the primary checkout's copy (its path under /mnt/c). Each clone is a
# standalone clone at ~/ois-wsl/<name> (`/` in the branch written as `-`) with its own
# .env, throwaway database (ois_wsl_ plus the name lower-cased, `-` as `_`), node_modules and
# target/. create, gate, push and pr refuse main and next.
#
# Usage (inside WSL; from Windows prefix `wsl.exe -d Ubuntu -- bash -lic '...'`):
#   bash bin/wsl-clone.sh preflight              # C: free space and WSL memory, against bin/lib/wsl.sh
#   bash bin/wsl-clone.sh create <branch>        # clone (or fast-forward) and provision
#   bash bin/wsl-clone.sh gate   <name|branch>   # fast-forward, preflight, `just test` in the clone
#   bash bin/wsl-clone.sh push   <name|branch>   # push, only with the clone's tests marker for HEAD
#   bash bin/wsl-clone.sh pr     <name|branch> [gh pr create args...]
#                                                # open the PR, only with the clone's tests marker
#                                                # and the Windows worktree's review marker for HEAD
#   bash bin/wsl-clone.sh remove <name|branch>   # drop the database, delete the clone, fstrim
#
# Settings and thresholds: bin/lib/wsl.sh. Tests: bin/lib/wsl-clone.test.sh.
# ==============================================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# The message is printed with %s, never echo -e: a Windows path's \e would become an escape.
fatal()   { printf "${RED}FATAL:${NC} %s\n" "$1" >&2; exit 1; }
blocked() { printf "${RED}BLOCKED:${NC} %s\n" "$1" >&2; exit 1; }
step()    { printf "\n${CYAN}==>${NC} %s\n" "$1"; }
ok()      { printf "  ${GREEN}ok${NC} %s\n" "$1"; }
warn()    { printf "  ${YELLOW}warn${NC} %s\n" "$1"; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# A Windows checkout under core.autocrlf holds portable.sh with CRLF endings, which bash on
# Linux cannot parse, so it is read with the CRs stripped. This file and wsl.sh are pinned
# to LF by bin/.gitattributes.
# shellcheck source=lib/portable.sh
. <(tr -d '\r' < "$SCRIPT_DIR/lib/portable.sh") || { echo "FATAL: cannot load ${SCRIPT_DIR}/lib/portable.sh" >&2; exit 1; }
# shellcheck source=lib/wsl.sh
. "$SCRIPT_DIR/lib/wsl.sh" || { echo "FATAL: cannot load ${SCRIPT_DIR}/lib/wsl.sh" >&2; exit 1; }

CLONES_ROOT="${OIS_WSL_CLONES:-$HOME/ois-wsl}"
DISTRO="${WSL_DISTRO_NAME:-}"
LXSS_KEY='HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss'

usage() { sed -n '/^# Usage/,/^# Settings/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 2; }

require_wsl() {
    grep -qi microsoft "${WSL_PROC_VERSION:-/proc/version}" 2>/dev/null \
        || fatal "not inside WSL. Run it there: wsl.exe -d Ubuntu -- bash -lic 'bash <this script> ...'"
}

# The main checkout this script sits in, on /mnt/c: the clone's source and its .env's.
# OIS_WSL_SOURCE names it instead, for a copy of this script outside that checkout (a
# branch changing the script, whose Windows worktree Linux git cannot open).
find_source() {
    local common
    if [[ -n "${OIS_WSL_SOURCE:-}" ]]; then
        common="$(git -C "$OIS_WSL_SOURCE" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
            || fatal "OIS_WSL_SOURCE=${OIS_WSL_SOURCE} is not a git checkout."
    else
        common="$(git -C "$SCRIPT_DIR" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
            || fatal "cannot find the checkout ${SCRIPT_DIR} belongs to. From a copy elsewhere, set OIS_WSL_SOURCE to the primary checkout."
    fi
    SOURCE="$(dirname "$common")"
}

# resolve NAME|BRANCH: sets NAME, DIR, DB.
resolve() {
    [[ -n "${1:-}" ]] || usage
    NAME="$(wsl_clone_name "$1")"
    [[ "$NAME" =~ ^[A-Za-z0-9_+-]+$ ]] || fatal "'$1' is not a clone name or branch."
    DIR="$CLONES_ROOT/$NAME"
    # Interpolated into SQL: wsl_db_name leaves nothing outside [a-z0-9_] in it.
    DB="$(wsl_db_name "$NAME")" || fatal "cannot derive the database name for ${NAME}."
}

clone_branch() {
    BRANCH="$(git -C "$DIR" symbolic-ref --quiet --short HEAD 2>/dev/null)" || fatal "${DIR} is not on a branch."
    SHA="$(git -C "$DIR" rev-parse HEAD)" || fatal "cannot read HEAD in ${DIR}."
}

# not_protected BRANCH: main and next are never pushed or gated here. master-protection-gate.sh
# refuses that push on Windows, and it cannot see a push run inside WSL.
not_protected() {
    case "$1" in
        main|next) blocked "${1} is a protected branch; this script works only on an issue branch." ;;
    esac
}

# The source's .env sits in a Windows checkout and may be CRLF; every read strips the CRs.
env_value() { tr -d '\r' < "$1" 2>/dev/null | grep -E "^$2=" | tail -1 | cut -d= -f2- | sed -E 's/[[:space:]]+#.*$//; s/^"(.*)"$/\1/'; }

# url_with_db URL DB: URL pointing at database DB, its query string kept.
url_with_db() {
    local base="${1%%\?*}"
    local rest="${base#*://}"
    [[ "$rest" == */* ]] && base="${base%/*}"
    printf '%s/%s%s\n' "$base" "$2" "${1#"${1%%\?*}"}"
}

# The source's DATABASE_URL, for creating and dropping the clone's database: a database
# cannot be dropped over a connection to itself.
use_admin_db() {
    DATABASE_URL="$(env_value "$SOURCE/.env" DATABASE_URL)"
    export DATABASE_URL
    [[ -n "$DATABASE_URL" ]] || blocked "${SOURCE}/.env sets no DATABASE_URL."
    db_reachable "$(url_part "$DATABASE_URL" host)" "$(url_part "$DATABASE_URL" port)" \
        || blocked "cannot reach Postgres at $(url_part "$DATABASE_URL" host):$(url_part "$DATABASE_URL" port). Start it (just up) and re-run."
}

db_exists() {
    local out
    out="$(db_query "select 1 from pg_database where datname = '${DB}'")" || blocked "cannot query Postgres (psql, or docker exec into its container)."
    [[ -n "$out" ]]
}

# native_tool NAME: refuses a missing tool, and Windows's copy reached through interop: the
# clone's node_modules must be installed by the Linux toolchain that runs its tests.
native_tool() {
    local path
    path="$(command -v "$1" 2>/dev/null)" || fatal "$1 not found in WSL. Install it there (fnm or nvm, then corepack enable) and run this through bash -lic."
    case "$path" in
        "$WSL_WINDOWS_MOUNT"/*) fatal "$1 resolves to Windows's copy (${path}). Install a native one in WSL and run this through bash -lic." ;;
    esac
    ok "$1: ${path}"
}

# sync_clone: fast-forward the clone to the Windows branch; never past local-only commits.
sync_clone() {
    git -C "$DIR" fetch -q windows || fatal "cannot fetch the clone's windows remote in ${DIR}."
    git -C "$DIR" merge-base --is-ancestor HEAD "windows/$BRANCH" 2>/dev/null \
        || blocked "the clone has commits windows/${BRANCH} lacks. Commit in the Windows worktree, never in the clone."
    git -C "$DIR" merge -q --ff-only "windows/$BRANCH" || fatal "cannot fast-forward ${DIR} to windows/${BRANCH}."
    ok "clone at $(git -C "$DIR" rev-parse --short HEAD) (windows/${BRANCH})"
}

clone_fresh() {
    local origin_url
    origin_url="$(git -C "$SOURCE" remote get-url origin)" || fatal "the source has no origin remote."
    mkdir -p "$CLONES_ROOT" || fatal "cannot create ${CLONES_ROOT}."
    git clone -q --no-checkout --origin windows "$SOURCE" "$DIR" || fatal "git clone of ${SOURCE} failed."
    git -C "$DIR" remote add origin "$origin_url" || fatal "cannot add origin to ${DIR}."
    git -C "$DIR" checkout -q -b "$BRANCH" --track "windows/$BRANCH" || fatal "cannot check out ${BRANCH} in ${DIR}."
    ok "cloned ${BRANCH} into ${DIR}"
}

# write_env: the source's .env, pointed at the clone's database, with the WSL build env.
write_env() {
    local src="$SOURCE/.env" url tmp
    url="$(env_value "$src" DATABASE_URL)"
    [[ -n "$url" ]] || fatal "${src} sets no DATABASE_URL."
    tmp="$DIR/.env.tmp"
    tr -d '\r' < "$src" | grep -vE "^(DATABASE_URL|CARGO_BUILD_JOBS|${WSL_RUSTFLAGS_VAR})=" > "$tmp"
    # Quoted: the file is sourced with `set -a`, where a bare `&` in a query string would fork.
    { echo "DATABASE_URL=\"$(url_with_db "$url" "$DB")\""; wsl_build_env_lines; } >> "$tmp"
    mv "$tmp" "$DIR/.env" || fatal "cannot write ${DIR}/.env."
    ok ".env: DATABASE_URL -> ${DB}; $(wsl_build_env_lines | tr '\n' ' ')"
}

create_db() {
    use_admin_db
    if db_exists; then
        ok "database ${DB} already exists"
        return 0
    fi
    db_query "create database \"${DB}\"" >/dev/null || true
    db_exists || blocked "database ${DB} does not exist after create."
    ok "database ${DB} created"
}

# vhdx_path: Windows path of this distro's ext4.vhdx, from the Lxss registry key.
vhdx_path() {
    reg.exe query "$LXSS_KEY" /s 2>/dev/null | tr -d '\r' | awk -v d="$DISTRO" '
        function flush() { if (name == d && base != "") print base "\\" (file == "" ? "ext4.vhdx" : file) }
        /^HKEY_/ { flush(); name = ""; base = ""; file = ""; next }
        { v = $0; sub(/^[ \t]*[^ \t]+[ \t]+REG_[A-Z_]+[ \t]+/, "", v) }
        $1 == "DistributionName" { name = v }
        $1 == "BasePath" { sub(/^\\\\\?\\/, "", v); base = v }
        $1 == "VhdFileName" { file = v }
        END { flush() }'
}

# sparse_check: a sparse vhdx hands space freed inside WSL back to C: (after fstrim).
# Setting the flag refuses while the distro runs (WSL_E_DISTRO_NOT_STOPPED), so this only reports.
sparse_check() {
    local hint="From Windows, with nothing running in WSL: wsl --shutdown, then wsl --manage ${DISTRO:-<distro>} --set-sparse true"
    local vhd flag
    vhd="$(vhdx_path)"
    if [[ -z "$DISTRO" || -z "$vhd" ]]; then
        warn "cannot find this distro's disk in the registry, so its sparse flag is unchecked. ${hint}"
        return 0
    fi
    flag="$(fsutil.exe sparse queryflag "$vhd" 2>/dev/null | tr -d '\r')"
    case "$flag" in
        *"NOT set as sparse"*) warn "${vhd} is not sparse: space freed in WSL stays taken on C:. ${hint}" ;;
        *"set as sparse"*) ok "${vhd} is sparse" ;;
        *) warn "fsutil could not read the sparse flag of ${vhd}. ${hint}" ;;
    esac
}

cmd_preflight() { wsl_preflight; }

cmd_create() {
    BRANCH="${1:-}"
    [[ -n "$BRANCH" ]] || usage
    not_protected "$BRANCH"
    resolve "$BRANCH"
    step "preflight"
    wsl_preflight || blocked "below a preflight threshold; nothing was created."
    step "tools"
    native_tool node
    native_tool pnpm
    step "clone"
    git -C "$SOURCE" show-ref --verify --quiet "refs/heads/$BRANCH" || fatal "${SOURCE} has no branch ${BRANCH}."
    if [[ -d "$DIR/.git" ]]; then
        local want="$BRANCH"
        clone_branch
        [[ "$BRANCH" == "$want" ]] || blocked "${DIR} has ${BRANCH} checked out, not ${want}. Remove it first: bash ${SCRIPT_DIR}/wsl-clone.sh remove ${NAME}"
        sync_clone
    else
        clone_fresh
    fi
    git -C "$DIR" config core.hooksPath .githooks || fatal "cannot set core.hooksPath in ${DIR}."
    GIT_TERMINAL_PROMPT=0 with_timeout 60 git -C "$DIR" fetch -q origin \
        || warn "cannot fetch origin: origin/next is stale until the next fetch"
    step "environment"
    write_env
    create_db
    (cd "$DIR" && pnpm install --frozen-lockfile --prefer-offline) || fatal "pnpm install --frozen-lockfile failed in ${DIR}."
    ok "node_modules installed"
    step "disk"
    sparse_check
    printf "\n${GREEN}Clone ready:${NC} %s. Gate it: bash %s/wsl-clone.sh gate %s\n" "$DIR" "$SCRIPT_DIR" "$NAME"
}

cmd_gate() {
    resolve "${1:-}"
    [[ -d "$DIR/.git" ]] || fatal "no clone at ${DIR}. Run: bash ${SCRIPT_DIR}/wsl-clone.sh create <branch>"
    clone_branch
    not_protected "$BRANCH"
    sync_clone
    wsl_preflight || blocked "below a preflight threshold; the build did not start."
    cd "$DIR" && just test
}

# marker_vouched ROOT KIND LABEL: the marker for $SHA in ROOT, with its manifest line, the
# same test bin/check-marker.sh applies for the PR gates.
marker_vouched() {
    python3 -I "$SCRIPT_DIR/lib/marker_manifest.py" vouched "$1" "$2" "$SHA" \
        || blocked "$3 has not run for ${SHA:0:8} in $1."
    ok "$3 marker for ${SHA:0:8} in $1"
}

cmd_push() {
    resolve "${1:-}"
    [[ -d "$DIR/.git" ]] || fatal "no clone at ${DIR}."
    clone_branch
    not_protected "$BRANCH"
    marker_vouched "$DIR" tests "Tests (bash ${SCRIPT_DIR}/wsl-clone.sh gate ${NAME})"
    git -C "$DIR" push -u origin "$BRANCH" || fatal "push failed."
    [[ "$(GIT_TERMINAL_PROMPT=0 with_timeout 30 git -C "$DIR" ls-remote origin "refs/heads/$BRANCH" | awk '{ print $1 }')" == "$SHA" ]] \
        || fatal "origin's ${BRANCH} does not equal ${SHA:0:8} after the push."
    ok "origin/${BRANCH} is ${SHA:0:8}"
}

# windows_worktree BRANCH: the directory of the worktree with BRANCH checked out.
windows_worktree() {
    wsl_mount_path "$(git -C "$SOURCE" worktree list --porcelain 2>/dev/null | awk -v b="refs/heads/$1" '
        /^worktree / { wt = substr($0, 10) }
        $1 == "branch" && $2 == b { print wt; exit }')"
}

# pr_text_clean LABEL FILE: refuses FILE when attribution_hits, the check attribution-gate.sh
# runs on the Windows side and never sees here, finds a credit in it or cannot read it.
pr_text_clean() {
    [[ "$2" != - && -f "$2" && -r "$2" ]] || blocked "cannot read ${1}, so its attribution is unchecked; pass a readable file."
    attribution_hits "$2" >/dev/null
    case "$?" in
        1) return 0 ;;
        0) blocked "${1} carries an attribution line." ;;
        *) blocked "cannot read ${1}, so its attribution is unchecked." ;;
    esac
}

pr_arg_clean() {
    local tmp
    tmp="$(mktemp)" || fatal "mktemp failed."
    printf '%s\n' "$1" > "$tmp"
    pr_text_clean "an argument" "$tmp"
    rm -f "$tmp"
}

# pr_short_flags CLUSTER: one `-xyz` cluster, read as gh's flag parser reads it. Sets PR_NEXT to
# `value` or `file` when the cluster's last flag takes the next argument.
pr_short_flags() {
    local i c rest
    for (( i = 1; i < ${#1}; i++ )); do
        c="${1:i:1}"; rest="${1:i+1}"
        case "$c" in
            R|B|H) blocked "pr sets -${c} itself; drop it." ;;
            e|T|w) blocked "pr takes its text from --title, --body or --body-file only; drop -${c}." ;;
            F) if [[ -n "$rest" ]]; then pr_text_clean "$rest" "$rest"; else PR_NEXT=file; fi; return 0 ;;
            t|b|a|l|m|p|r) [[ -n "$rest" ]] || PR_NEXT=value; return 0 ;;
        esac
    done
}

# pr_args_allowed ARGS...: the script sets the repo, base and head itself, and the PR text
# carries no attribution.
pr_args_allowed() {
    local a
    PR_NEXT=""
    for a in "$@"; do
        case "$PR_NEXT" in
            file) PR_NEXT=""; pr_text_clean "$a" "$a"; continue ;;
            value) PR_NEXT=""; pr_arg_clean "$a"; continue ;;
        esac
        case "$a" in
            --repo|--repo=*|--base|--base=*|--head|--head=*) blocked "pr sets ${a%%=*} itself; drop it." ;;
            --editor|--template|--template=*|--web) blocked "pr takes its text from --title, --body or --body-file only; drop ${a%%=*}." ;;
            --body-file) PR_NEXT=file ;;
            --body-file=*) pr_text_clean "${a#*=}" "${a#*=}" ;;
            --title|--body|--assignee|--label|--milestone|--project|--reviewer|--recover) PR_NEXT=value ;;
            --*) ;;
            -?*) pr_short_flags "$a" ;;
        esac
        pr_arg_clean "$a"
    done
    [[ -z "$PR_NEXT" ]] || blocked "the last flag has no value."
}

cmd_pr() {
    resolve "${1:-}"
    shift || true
    [[ -d "$DIR/.git" ]] || fatal "no clone at ${DIR}."
    clone_branch
    not_protected "$BRANCH"
    # The primary checkout's copy of the hooks' own pattern, as the hooks themselves run it.
    # shellcheck source=../.claude/hooks/lib/attribution.sh
    . <(tr -d '\r' < "$SOURCE/.claude/hooks/lib/attribution.sh" 2>/dev/null) && declare -F attribution_hits >/dev/null \
        || blocked "cannot load ${SOURCE}/.claude/hooks/lib/attribution.sh, so the PR text cannot be checked."
    pr_args_allowed "$@"
    local wt
    wt="$(windows_worktree "$BRANCH")"
    [[ -n "$wt" && -d "$wt" ]] || blocked "no Windows worktree has ${BRANCH} checked out, so its review marker cannot be read."
    [[ "$(git -C "$SOURCE" rev-parse "refs/heads/$BRANCH" 2>/dev/null)" == "$SHA" ]] \
        || blocked "the clone's HEAD ${SHA:0:8} is not the Windows branch's. Run: bash ${SCRIPT_DIR}/wsl-clone.sh gate ${NAME}"
    # gh opens the PR on origin's branch, so that is the commit the markers must cover.
    [[ "$(GIT_TERMINAL_PROMPT=0 with_timeout 30 git -C "$DIR" ls-remote origin "refs/heads/$BRANCH" | awk '{ print $1 }')" == "$SHA" ]] \
        || blocked "origin's ${BRANCH} is not ${SHA:0:8}. Run: bash ${SCRIPT_DIR}/wsl-clone.sh push ${NAME}"
    marker_vouched "$DIR" tests "Tests (bash ${SCRIPT_DIR}/wsl-clone.sh gate ${NAME})"
    marker_vouched "$wt" review-shipping "Review (/review-before-shipping)"
    (cd "$DIR" && gh pr create --repo VATUSA/OIS --base next --head "$BRANCH" "$@")
}

drop_db() {
    use_admin_db
    if ! db_exists; then
        ok "database ${DB} not present"
        return 0
    fi
    db_query "drop database \"${DB}\"" >/dev/null || true
    # Read back, so a drop that did not take (a backend still connected) fails here.
    db_exists && blocked "${DB} still exists after the drop (a backend still connected to it?). Stop it and re-run."
    ok "database ${DB} dropped"
}

# The clone's commits must be on origin: its pushed branch at HEAD, or origin/next.
check_saved() {
    local status
    status="$(git -C "$DIR" status --porcelain)" || blocked "git status failed in ${DIR}, so its changes are unknown."
    [[ -z "$status" ]] || blocked "${DIR} has uncommitted or untracked changes."
    clone_branch
    GIT_TERMINAL_PROMPT=0 with_timeout 60 git -C "$DIR" fetch -q --prune origin \
        || blocked "cannot fetch origin, so whether ${SHA:0:8} is saved there is unknown."
    local remote
    remote="$(GIT_TERMINAL_PROMPT=0 with_timeout 30 git -C "$DIR" ls-remote origin "refs/heads/$BRANCH" | awk '{ print $1 }')"
    if [[ "$remote" != "$SHA" ]] && ! git -C "$DIR" merge-base --is-ancestor "$SHA" origin/next 2>/dev/null; then
        blocked "${BRANCH} at ${SHA:0:8} is neither on origin/${BRANCH} nor in origin/next. Push it first."
    fi
    ok "${SHA:0:8} is on origin"
}

cmd_remove() {
    resolve "${1:-}"
    if [[ -d "$DIR" ]]; then
        check_saved
    else
        ok "no clone at ${DIR}"
    fi
    drop_db
    rm -rf "$DIR"
    [[ ! -e "$DIR" ]] || blocked "${DIR} is still there after rm -rf."
    ok "removed ${DIR}"
    # Hand the freed blocks back to the vhdx, so a sparse disk shrinks on C:.
    if [[ -n "$DISTRO" ]] && wsl.exe -d "$DISTRO" -u root fstrim -v /; then
        ok "fstrim done"
        return 0
    fi
    printf "${RED}fstrim did not run; the space stays taken on C:.${NC} From Windows: wsl.exe -d %s -u root fstrim -v /\n" "${DISTRO:-<distro>}" >&2
    return 1
}

SUBCOMMAND="${1:-}"
shift || true
case "$SUBCOMMAND" in
    preflight|create|gate|push|pr|remove) ;;
    *) usage ;;
esac
require_wsl
case "$SUBCOMMAND" in create|pr|remove) find_source ;; esac
# One line, ending in exit: bash reads a script as it runs, so a checkout updated during a
# long gate would otherwise feed it the new file's bytes from the old offset.
"cmd_$SUBCOMMAND" "$@"; exit $?
