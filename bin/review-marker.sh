#!/bin/bash
# Writes the SHA-keyed review markers for the commit that was reviewed.
#
#   bash bin/review-marker.sh code-review <reviewed-sha>   # /code-review Phase 9
#   bash bin/review-marker.sh shipping <reviewed-sha>      # /review-before-shipping Phase 9
#
# `code-review` writes .claude/markers/code-review/<sha>; `shipping` writes that and
# .claude/markers/review-shipping/<sha>, the one pre-pr-gate.sh checks. Each gets its
# manifest line (bin/lib/marker_manifest.py), without which a marker is not a pass.
#
# A script because the guard needs git inside $(...), which the harness refuses
# from a worktree session, so the block could not run where reviews happen.
#
# Refuses unless HEAD still equals the reviewed SHA, the checkout is clean and every
# review lens attested every rule heading it owns for that SHA: the marker records a
# review of that exact commit, never a bypass.

KIND="${1:-}"
REVIEWED_SHA="${2:-}"
case "$KIND" in
    code-review) TYPES="code-review" ;;
    shipping) TYPES="code-review review-shipping" ;;
    *)
        echo "usage: bash bin/review-marker.sh <code-review|shipping> <reviewed-sha>" >&2
        exit 2
        ;;
esac
if [[ ! "$REVIEWED_SHA" =~ ^[0-9a-f]{40}$ ]]; then
    echo "BLOCKER: pass the full 40-character SHA pinned in Phase 0." >&2
    exit 1
fi

CURRENT_SHA="$(git rev-parse HEAD)" || { echo "BLOCKER: cannot resolve HEAD." >&2; exit 1; }
if [[ "$CURRENT_SHA" != "$REVIEWED_SHA" ]]; then
    echo "BLOCKER: HEAD moved during review ($REVIEWED_SHA -> $CURRENT_SHA) — re-run the review against the current commit." >&2
    exit 1
fi
STATUS="$(git status --porcelain --untracked-files=normal)" || { echo "BLOCKER: git status failed." >&2; exit 1; }
if [[ -n "$STATUS" ]]; then
    echo "BLOCKER: checkout is dirty — commit every reviewed change before writing the marker." >&2
    exit 1
fi

ATTESTATION="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/review-attestation.sh"
if [[ ! -f "$ATTESTATION" ]]; then
    echo "BLOCKER: cannot locate bin/lib/review-attestation.sh — refusing to write an unattested marker." >&2
    exit 1
fi
if ! bash "$ATTESTATION" check "$REVIEWED_SHA" >&2; then
    echo "BLOCKER: lens attestation incomplete — re-dispatch the lenses named above (/code-review Phase 3)." >&2
    exit 1
fi

# The checkout's own .claude, the only place pre-pr-gate.sh looks (through check-marker.sh,
# at the checkout's toplevel; there is no main-checkout fallback). A failed write must not
# report success.
TOPLEVEL="$(git rev-parse --show-toplevel)" || { echo "BLOCKER: cannot resolve the checkout root." >&2; exit 1; }
MANIFEST_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/marker_manifest.py"
for TYPE in $TYPES; do
    MARKER="$TOPLEVEL/.claude/markers/$TYPE/$REVIEWED_SHA"
    python3 -I "$MANIFEST_LIB" stamp "$TOPLEVEL" "$TYPE" "$REVIEWED_SHA" \
        || { echo "BLOCKER: could not write $MARKER." >&2; exit 1; }
    echo "Review marker written for ${REVIEWED_SHA:0:12}: $MARKER"
done
