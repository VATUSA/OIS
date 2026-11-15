#!/bin/bash
# ==============================================================================
# Delete build output no build has written for DAYS days, in every checkout's target/.
#
#   bin/clean-stale.sh [--scratch] [DAYS]        (just clean-stale [--scratch] [DAYS])
#
# DAYS defaults to 7. Checkouts are the primary and every `git worktree list` entry.
# In each <checkout>/target/ it deletes, when nothing inside was modified in the last
# DAYS days:
#   <profile>/build/<pkg>/<hash>/    one cargo unit (then <pkg>/ once it is empty)
#   <profile>/incremental/<dir>/     rustc's incremental state for one crate
#   ois-drift/<dir>/                 bin/run-tests.sh's client-drift spec
# Age is the newest mtime, not atime: /home is mounted noatime here, and cargo writes
# nothing to a unit it reuses, so a unit still current but not rebuilt for DAYS days
# goes too and costs one rebuild.
#
# A running build holds an exclusive flock on <profile>/.cargo-build-lock. This
# script takes that lock with `flock -n` and holds it while it deletes the profile's
# units, so it never deletes under cargo's build and no build starts mid-delete. When
# the lock is held, cannot be opened, or flock is not installed, the profile is kept.
# The lock does not cover a test run: cargo releases it once the build is done, and
# nextest then runs test binaries from build/<pkg>/<hash>/out/. A unit DAYS old that a
# running suite has yet to spawn can be deleted, and that test fails to start.
#
# The unit layout is the one this repo's nightly writes (build/<pkg>/<hash>/, no deps/).
# A profile with a deps/ or .fingerprint/ dir is cargo's older layout, where a
# build/<pkg>-<hash>/ entry is not a whole unit; it is kept untouched.
#
# --scratch also deletes ${TMPDIR:-/tmp}/mutate.*/ dirs (bin/mutate.sh's backup and
# log; a failed restore leaves its backup there) older than DAYS. Nothing else.
#
# Exit 0 done, 1 some directory could not be deleted (each is named), 2 bad arguments.
# Tests: bin/lib/clean-stale.test.sh.
# ==============================================================================
set -uo pipefail

usage() { echo "usage: bin/clean-stale.sh [--scratch] [DAYS]   (DAYS a positive integer, default 7)" >&2; exit 2; }

SCRATCH=0 DAYS=7 DAYS_GIVEN=0
for arg in "$@"; do
    case "$arg" in
        --scratch) SCRATCH=1 ;;
        -h|--help) usage ;;
        *) [ "$DAYS_GIVEN" = 0 ] || usage; DAYS="$arg" DAYS_GIVEN=1 ;;
    esac
done
case "$DAYS" in ''|0*|*[!0-9]*) usage ;; esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/portable.sh
. "$SCRIPT_DIR/lib/portable.sh"
ROOT="$(main_repo_root)" || { echo "clean-stale: not inside a git repository" >&2; exit 2; }

FAILED=0

# stale DIR: true when neither DIR nor anything under it was modified in the last DAYS
# days. A find error (an unreadable subdir) counts as fresh, so the dir is kept.
stale() {
    local newer
    newer="$(find "$1" -mtime "-$DAYS" -print -quit 2>/dev/null)" || return 1
    [ -z "$newer" ]
}

# remove DIR: deletes DIR, adds its size to FREED_KB; names it and sets FAILED when rm fails.
remove() {
    local kb
    kb="$(du -sk "$1" 2>/dev/null | cut -f1)"
    if rm -rf "$1"; then
        FREED_KB=$(( FREED_KB + ${kb:-0} )) REMOVED=$(( REMOVED + 1 ))
    else
        echo "clean-stale: could not delete $1" >&2
        FAILED=1
    fi
}

# clean_units PROFILE: the deletion that runs while the profile's build lock is held.
clean_units() {
    local unit pkg
    for unit in "$1"/build/*/*/ "$1"/incremental/*/; do
        [ -d "$unit" ] && stale "$unit" && remove "${unit%/}"
    done
    for pkg in "$1"/build/*/; do
        [ -d "$pkg" ] && [ -z "$(ls -A "$pkg")" ] && remove "${pkg%/}"
    done
    return 0
}

# clean_profile PROFILE: clean_units under `flock -n -x` on cargo's build lock, or keep.
clean_profile() {
    local before=$REMOVED kb_before=$FREED_KB counts
    if ! command -v flock >/dev/null 2>&1; then
        echo "  kept $1: flock is not installed, so a running build cannot be ruled out"
        return 0
    fi
    # The lock lives as long as fd 9, so the subshell holds it for the whole deletion.
    # Its counters do not reach this shell, so it prints them; nothing printed means the
    # lock file could not be opened or the lock was not taken.
    counts="$( (
        flock -n -x 9 || exit 1
        clean_units "$1"
        echo "$REMOVED $FREED_KB $FAILED"
    ) 9>>"$1/.cargo-build-lock" )"
    if [ -z "$counts" ]; then
        echo "  kept $1: a running build holds .cargo-build-lock, or the lock could not be taken"
        return 0
    fi
    read -r REMOVED FREED_KB FAILED <<< "$counts"
    echo "  $1: removed $(( REMOVED - before )) dirs, $(( (FREED_KB - kb_before) / 1024 )) MB"
}

clean_target() {
    local profile drift before=$REMOVED kb_before=$FREED_KB
    echo "$1"
    for profile in "$1"/*/; do
        profile="${profile%/}"
        [ -d "$profile/build" ] || [ -d "$profile/incremental" ] || continue
        if [ -d "$profile/deps" ] || [ -d "$profile/.fingerprint" ]; then
            echo "  kept $profile: cargo's older layout (deps/ or .fingerprint/), whose units this script cannot tell apart"
            continue
        fi
        clean_profile "$profile"
    done
    for drift in "$1"/ois-drift/*/; do
        [ -d "$drift" ] && stale "$drift" && remove "${drift%/}"
    done
    echo "  total: removed $(( REMOVED - before )) dirs, $(( (FREED_KB - kb_before) / 1024 )) MB"
}

REMOVED=0 FREED_KB=0
echo "clean-stale: deleting build output untouched for ${DAYS} days"
LISTING="$(git -C "$ROOT" worktree list --porcelain)" || { echo "clean-stale: git worktree list failed" >&2; exit 2; }
while IFS= read -r checkout; do
    [ -d "$checkout/target" ] && clean_target "$checkout/target"
done <<EOF
$(printf '%s\n' "$LISTING" | sed -n 's/^worktree //p')
EOF

if [ "$SCRATCH" = 1 ]; then
    before=$REMOVED kb_before=$FREED_KB
    for dir in "${TMPDIR:-/tmp}"/mutate.*/; do
        [ -d "$dir" ] && stale "$dir" && remove "${dir%/}"
    done
    echo "${TMPDIR:-/tmp}/mutate.*: removed $(( REMOVED - before )) dirs, $(( (FREED_KB - kb_before) / 1024 )) MB"
fi

echo "clean-stale: removed $REMOVED dirs, $(( FREED_KB / 1024 )) MB in all"
exit "$FAILED"
