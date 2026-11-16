#!/bin/bash
# ==============================================================================
# Guard: rust-toolchain.toml names a dated nightly, and every dtolnay/rust-toolchain step in
# .github/workflows/*.yml installs that same toolchain (#1044).
#
#   bash bin/lib/toolchain-pin.test.sh
#
# A source scan, because the workflows only run on GitHub and release.yml only on a tag: a
# date bumped in the toml but missed in one workflow passes every PR check and breaks the
# release build. The negative cases run the same check against planted copies.
# Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/toolchain-pin-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# check DIR: prints one line per problem in DIR's rust-toolchain.toml and workflows, then
# `steps=N`. A `dtolnay/rust-toolchain@` step's `toolchain:` is read from the lines after it,
# up to the next step at the same indent.
check() {
    local dir="$1" channel steps=0
    channel="$(sed -nE 's/^channel = "([^"]*)"$/\1/p' "$dir/rust-toolchain.toml")"
    [[ "$channel" =~ ^nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] \
        || echo "rust-toolchain.toml channel '$channel' is not a dated nightly"
    for wf in "$dir"/.github/workflows/*.yml; do
        while IFS='|' read -r line ref tc; do
            steps=$((steps + 1))
            [[ "$ref" == "master" ]] || echo "$(basename "$wf"):$line uses @$ref, not @master"
            [[ "$tc" == "$channel" ]] || echo "$(basename "$wf"):$line installs '$tc', not '$channel'"
        done < <(awk '
            function flush() { if (on) print start "|" ref "|" tc; on = 0 }
            match($0, /^ *- uses: dtolnay\/rust-toolchain@/) {
                flush(); on = 1; start = NR; tc = ""
                ind = RLENGTH - length("- uses: dtolnay/rust-toolchain@")
                ref = substr($0, RLENGTH + 1); next
            }
            on && match($0, /^ *- /) && RLENGTH - 2 <= ind { flush() }
            on && /^ +toolchain: / { tc = $2 }
            END { flush() }' "$wf")
    done
    echo "steps=$steps"
}

echo "this repository:"
OUT="$(check "$ROOT")"
STEPS="$(sed -n 's/^steps=//p' <<<"$OUT")"
PROBLEMS="$(grep -v '^steps=' <<<"$OUT")"
[[ -z "$PROBLEMS" ]] && ok "the toml names a dated nightly and every workflow step installs it" \
    || bad "toolchain mismatch:"$'\n'"$PROBLEMS"
(( STEPS >= 5 )) && ok "found $STEPS dtolnay/rust-toolchain steps (at least 5)" \
    || bad "found only $STEPS dtolnay/rust-toolchain steps: the scan is not matching"

# plant NAME: a copy of the toml and workflows to break. Sets COPY.
plant() {
    COPY="$WORK/$1"
    mkdir -p "$COPY/.github"
    cp "$ROOT/rust-toolchain.toml" "$COPY/"
    cp -r "$ROOT/.github/workflows" "$COPY/.github/"
}

echo "planted mismatches:"
plant rolling
sed -i -E 's/^channel = ".*"$/channel = "nightly"/' "$COPY/rust-toolchain.toml"
grep -q "is not a dated nightly" <<<"$(check "$COPY")" \
    && ok "a rolling nightly channel is refused" || bad "a rolling channel passed"

plant stale-release
sed -i -E '0,/^( +)toolchain: nightly-.*/s//\1toolchain: nightly-2000-01-01/' "$COPY/.github/workflows/release.yml"
grep -q "release.yml:.* installs 'nightly-2000-01-01'" <<<"$(check "$COPY")" \
    && ok "one workflow step on another date is refused, naming the file" || bad "a stale date passed"

plant branch-ref
sed -i -E '0,/dtolnay\/rust-toolchain@master/s//dtolnay\/rust-toolchain@nightly/' "$COPY/.github/workflows/ci.yml"
grep -q "ci.yml:.* uses @nightly, not @master" <<<"$(check "$COPY")" \
    && ok "a step back on the @nightly branch is refused" || bad "an @nightly step passed"

echo ""
echo "toolchain-pin: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
