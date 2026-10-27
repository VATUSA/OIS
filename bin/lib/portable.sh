#!/bin/bash
# Shared shell helpers. Sourced, never executed; must stay side-effect-free on source.
# Bash 3.2 and BSD/GNU safe.

# main_repo_root
#
# Absolute path of the MAIN checkout, correct even from inside a worktree.
# `--show-toplevel` would return the worktree, and `$PWD` returns wherever the
# caller happens to stand.
#
# Returns 1 and prints nothing outside a git repository, so callers fail loudly
# instead of building paths off an empty root.
main_repo_root() {
    local common
    common="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || return 1
    [[ -n "$common" ]] || return 1
    dirname "$common"
}

# export_build_env
#
# Exports RUSTC_WRAPPER=sccache when sccache is on PATH and the caller set no wrapper.
# sccache is optional: without it nothing is exported and nothing fails. The wrapper is
# the bare name everywhere (justfile, SessionStart hook, these scripts), so cargo sees
# one value and does not rebuild over a spelling difference. CARGO_TARGET_DIR is never
# set: each worktree builds into its own target/, which bin/setup-worktree.sh seeds from
# the primary checkout's. One target dir shared across worktrees ran another worktree's
# stale test binary, because cargo hashes path packages workspace-relative and judges
# their freshness by mtime.
export_build_env() {
    if [[ -z "${RUSTC_WRAPPER+set}" ]] && command -v sccache >/dev/null 2>&1; then
        export RUSTC_WRAPPER=sccache
    fi
    return 0
}

# worktree_db_name [DIR]
#
# The throwaway database a linked worktree may create to boot its own backend
# (.claude/rules/database-postgres.md -> "Running against a real database"):
# ois_wt_<id>, where <id> is the worktree's administrative name under
# .git/worktrees/: the basename of its directory when it was added (an
# EnterWorktree worktree's .claude/worktrees/<NAME>), fixed for the worktree's
# life, unique among live worktrees, and unchanged when /start renames the branch.
# bin/setup-worktree.sh prints it, bin/verify-worktree.sh reports it,
# bin/teardown-worktree.sh drops it. Returns 1 for the main checkout (which has no
# throwaway database) or outside git.
worktree_db_name() {
    local dir="${1:-.}" gitdir common
    gitdir="$(git -C "$dir" rev-parse --path-format=absolute --git-dir 2>/dev/null)" || return 1
    common="$(git -C "$dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || return 1
    [[ -n "$gitdir" && "$gitdir" != "$common" ]] || return 1
    worktree_db_name_for_id "$(basename "$gitdir")"
}

# worktree_db_name_for_id ID: the same name from the id alone, for a worktree whose
# directory is already gone. Lower-cased, every character outside [a-z0-9_] folded to
# `_`, cut to Postgres's 63-byte identifier limit. Returns 1 for an empty id.
worktree_db_name_for_id() {
    local id
    id="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | LC_ALL=C sed 's/[^a-z0-9_]/_/g')"
    [[ -n "$id" ]] || return 1
    printf 'ois_wt_%s\n' "$id" | cut -c1-63
}

# url_part URL FIELD: user, host, port or db from a postgres://user:pass@host:port/db URL.
url_part() {
    local rest="${1#*://}" auth="" hostport path
    case "$rest" in *@*) auth="${rest%%@*}"; rest="${rest#*@}" ;; esac
    hostport="${rest%%/*}"
    path="${rest#"$hostport"}"; path="${path#/}"; path="${path%%\?*}"
    case "$2" in
        user) printf '%s' "${auth%%:*}" ;;
        host) printf '%s' "${hostport%%:*}" ;;
        port) case "$hostport" in *:*) printf '%s' "${hostport##*:}" ;; *) printf '5432' ;; esac ;;
        db) printf '%s' "$path" ;;
    esac
}

# db_query SQL: runs SQL against $DATABASE_URL with psql on PATH, else with psql inside
# the Docker container publishing the URL's port (compose's ois-postgres-1; psql is not
# installed on the host by default); prints rows unaligned. Returns 2 when neither is
# available, the query's status otherwise.
db_query() {
    local container
    if command -v psql >/dev/null 2>&1; then
        psql "$DATABASE_URL" -Atqc "$1" 2>/dev/null
        return
    fi
    command -v docker >/dev/null 2>&1 || return 2
    container="$(docker ps --format '{{.Names}} {{.Ports}}' 2>/dev/null \
        | awk -v p=":$(url_part "$DATABASE_URL" port)->" 'index($0, p) { print $1; exit }')"
    [[ -n "$container" ]] || return 2
    docker exec -i "$container" psql -U "$(url_part "$DATABASE_URL" user)" -d "$(url_part "$DATABASE_URL" db)" -Atqc "$1" 2>/dev/null
}

# db_reachable HOST PORT: pg_isready when installed, else a TCP connect (nc, or bash's
# /dev/tcp), 3 s at most. Perl's IO::Socket::INET is not used: with a Timeout it reports
# a refused connection on macOS as connected.
db_reachable() {
    if command -v pg_isready >/dev/null 2>&1; then
        pg_isready -q -h "$1" -p "$2" -t 3
        return
    fi
    if command -v nc >/dev/null 2>&1; then
        nc -z -w 3 "$1" "$2" >/dev/null 2>&1
        return
    fi
    with_timeout 3 bash -c 'exec 3<>"/dev/tcp/$1/$2"' _ "$1" "$2" 2>/dev/null
}

# with_timeout SECONDS COMMAND...: COMMAND, killed after SECONDS. timeout(1) is not on
# macOS by default; perl's alarm survives the exec and does the same.
with_timeout() {
    local seconds="$1"
    shift
    if command -v timeout >/dev/null 2>&1; then
        timeout "$seconds" "$@"
    else
        perl -e 'alarm shift; exec @ARGV or exit 127' "$seconds" "$@"
    fi
}
