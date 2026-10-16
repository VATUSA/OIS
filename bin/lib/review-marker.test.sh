#!/bin/bash
# ==============================================================================
# Tests for bin/review-marker.sh.
#
#   bash bin/lib/review-marker.test.sh
#
# Copies the script and the attestation library into a scratch repository with a
# one-lens map, then checks which states write which markers. Exits non-zero on
# any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/review-marker-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REPO="$WORK/repo"
mkdir -p "$REPO/bin/lib" "$REPO/.claude" "$REPO/rules"
cp "$ROOT/bin/review-marker.sh" "$REPO/bin/review-marker.sh"
cp "$ROOT/bin/lib/review-attestation.sh" "$REPO/bin/lib/review-attestation.sh"
cp "$ROOT/bin/lib/marker_manifest.py" "$REPO/bin/lib/marker_manifest.py"
vouched() { python3 -I "$ROOT/bin/lib/marker_manifest.py" vouched "$@"; }
printf 'code\trules/a.md\n' > "$REPO/.claude/review-lenses.tsv"
printf '## Alpha\n' > "$REPO/rules/a.md"
printf '.claude/markers/\n' > "$REPO/.gitignore"
cd "$REPO" || exit 1
git init -q . && git config user.email t@example.com && git config user.name t
git add . && git commit -qm init
SHA="$(git rev-parse HEAD)"
M="$REPO/.claude/markers"

attest() { mkdir -p "$M/attest/$SHA" && printf '| rules/a.md | Alpha | Checked | x.rs:1 |\n' > "$M/attest/$SHA/code.md"; }
reset() { rm -rf "$M"; }

# run WANT NAME ARGS...
run() {
    local want="$1" name="$2" got; shift 2
    (cd "$REPO" && bash bin/review-marker.sh "$@" >"$WORK/out" 2>&1)
    got=$?
    if [[ "$got" == "$want" ]]; then ok "$name"; else bad "$name (want $want, got $got): $(head -2 "$WORK/out")"; fi
}

echo "Arguments:"
reset; run 2 "an unknown kind is a usage error" review "$SHA"
reset; attest; run 1 "a short SHA is refused" code-review "${SHA:0:12}"
grep -q 'full 40-character SHA' "$WORK/out" && ok "...by the full-SHA check, not a later one" || bad "short-SHA message: $(head -1 "$WORK/out")"
[[ ! -e "$M/code-review" ]] && ok "...and writes no marker" || bad "a marker was written for a short SHA"

echo "Writes:"
reset; attest; run 0 "code-review with a complete attestation" code-review "$SHA"
[[ -f "$M/code-review/$SHA" && ! -e "$M/review-shipping" ]] && ok "...writes the code-review marker only" || bad "code-review wrote the wrong markers"
reset; attest; run 0 "shipping with a complete attestation" shipping "$SHA"
[[ -f "$M/code-review/$SHA" && -f "$M/review-shipping/$SHA" ]] && ok "...writes both markers" || bad "shipping did not write both markers"
vouched "$REPO" code-review "$SHA" && vouched "$REPO" review-shipping "$SHA" \
    && ok "...each with its manifest line" || bad "a marker has no manifest line: $(cat "$M/manifest" 2>&1)"
[[ "$(wc -l < "$M/manifest")" -eq 2 && -z "$(ls -A "$M" | grep stamp)" ]] && ok "...and nothing else" \
    || bad "manifest or temp files left over: $(ls -A "$M")"

echo "Refuses:"
reset; run 1 "no attestation" shipping "$SHA"
[[ ! -e "$M/review-shipping" ]] && ok "...and writes no marker" || bad "a marker was written without attestation"
reset; attest; printf 'dirty\n' > "$REPO/untracked.txt"
run 1 "an untracked file makes the checkout dirty" shipping "$SHA"
rm "$REPO/untracked.txt"
reset; attest; printf '## Alpha\n## Beta\n' > rules/a.md && git commit -qam beta
run 1 "HEAD moved past the reviewed SHA" shipping "$SHA"
grep -q 'HEAD moved' "$WORK/out" && ok "...and says HEAD moved" || bad "moved-HEAD message: $(head -1 "$WORK/out")"
SHA="$(git rev-parse HEAD)"
reset; attest; run 1 "a heading added at the reviewed commit is still required" shipping "$SHA"
[[ ! -e "$M/review-shipping" ]] && ok "...and writes no marker" || bad "a marker was written over a gap"

cd "$ROOT" || exit 1
echo ""
echo "review-marker: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
