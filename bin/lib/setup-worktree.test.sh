#!/bin/bash
# ==============================================================================
# Tests for bin/setup-worktree.sh's Cargo target seeding: a worktree with no
# target/ gets a copy-on-write clone of the primary checkout's, and every tracked
# file is touched right after it, so cargo rebuilds each workspace crate instead
# of trusting a seeded artifact built from the primary's source.
#
#   bash bin/lib/setup-worktree.test.sh
#
# Runs the script on a linked worktree of a scratch repository with stub pnpm,
# psql and pg_isready first on PATH, and a cp stub that logs the clone call and
# performs it with `cp -Rp` (so the seeded artifact keeps its old mtime), or
# fails it after a partial copy. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/bin/setup-worktree.sh"
WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/setup-worktree-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR CARGO_TARGET_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REAL_CP="$(command -v cp)"
mkdir -p "$WORK/bin"
printf '#!/bin/bash\nexit 0\n' > "$WORK/bin/pnpm"
printf '#!/bin/bash\nexit 0\n' > "$WORK/bin/pg_isready"
printf '#!/bin/bash\nexit 1\n' > "$WORK/bin/psql"
# cp: a clone (-c -R on macOS, --reflink=always on Linux) is logged, then copied with
# -Rp, or with $STUB_CLONE_FAIL set leaves a partial copy and fails; any other cp is real.
cat > "$WORK/bin/cp" <<STUB
#!/bin/bash
case " \$* " in
    *" -c -R "*|*" --reflink=always "*)
        echo "clone \$*" >> "$WORK/cp.log"
        src="\${@: -2:1}" dst="\${@: -1}"
        if [[ -n "\${STUB_CLONE_FAIL:-}" ]]; then mkdir -p "\$dst/partial"; exit 1; fi
        exec "$REAL_CP" -Rp "\$src" "\$dst" ;;
esac
exec "$REAL_CP" "\$@"
STUB
chmod +x "$WORK/bin/"*

MAIN="$WORK/main"
git init -q -b next "$MAIN"
mkdir -p "$MAIN/crates/a/src"
printf 'pub fn a() {}\n' > "$MAIN/crates/a/src/lib.rs"
printf 'gone\n' > "$MAIN/crates/a/src/deleted.rs"
printf 'target/\n.env\nnode_modules/\n.claude/\n' > "$MAIN/.gitignore"
printf 'lockfileVersion: 9.0\n' > "$MAIN/pnpm-lock.yaml"
git -C "$MAIN" add -A && git -C "$MAIN" -c core.hooksPath=/dev/null commit -qm init
printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\n' > "$MAIN/.env"
# The primary's build output, dated 2001.
mkdir -p "$MAIN/target/debug/deps"
printf 'artifact\n' > "$MAIN/target/debug/deps/liba.rlib"
touch -t 200101010000 "$MAIN/target/debug/deps/liba.rlib"

# new_wt NAME: a fresh linked worktree whose tracked source is dated 2000, older than the seed.
new_wt() {
    WT="$WORK/wt-$1"
    git -C "$MAIN" worktree add -q -b "feat/1/$1-case" "$WT" 2>/dev/null
    touch -t 200001010000 "$WT/crates/a/src/lib.rs"
}
# setup -> runs the script on $WT; sets OUT
setup() {
    : > "$WORK/cp.log"
    OUT="$(PATH="$WORK/bin:$PATH" bash "$SCRIPT" "$WT" 2>&1)"
}
mtime() { if stat -f %m "$1" >/dev/null 2>&1; then stat -f %m "$1"; else stat -c %Y "$1"; fi; }
has() { case "$OUT" in *"$2"*) ok "$1" ;; *) bad "$1 (no '$2' in: $OUT)" ;; esac; }

echo "No target/ in the worktree: seeded, then the sources touched:"
new_wt seed
rm "$WT/crates/a/src/deleted.rs"
setup
case "$(uname -s)" in Darwin) want="clone -c -R $MAIN/target $WT/target" ;; *) want="clone -R --reflink=always $MAIN/target $WT/target" ;; esac
[[ "$(cat "$WORK/cp.log")" == "$want" ]] && ok "clones the primary's target/ copy-on-write" || bad "clone call: $(cat "$WORK/cp.log")"
[[ -f "$WT/target/debug/deps/liba.rlib" ]] && ok "...into the worktree's target/" || bad "no seeded artifact"
has "...and says how long it took" "seeded target/ from $MAIN/target in "
src="$(mtime "$WT/crates/a/src/lib.rs")" art="$(mtime "$WT/target/debug/deps/liba.rlib")"
[[ "$src" -gt "$art" ]] && ok "every tracked source is newer than every seeded artifact" || bad "source mtime $src not after artifact $art"
[[ ! -e "$WT/crates/a/src/deleted.rs" ]] && ok "...and a tracked file deleted in the worktree is not recreated" || bad "the touch recreated a deleted tracked file"

echo "A target/ already there is left alone:"
new_wt present
mkdir -p "$WT/target" && printf 'mine\n' > "$WT/target/own"
setup
[[ ! -s "$WORK/cp.log" ]] && ok "no clone" || bad "cloned over an existing target/: $(cat "$WORK/cp.log")"
[[ "$(cat "$WT/target/own")" == mine && ! -e "$WT/target/debug" ]] && ok "...and its contents are untouched" || bad "target/ changed"
[[ "$(mtime "$WT/crates/a/src/lib.rs")" -lt 978307200 ]] && ok "...and no source is touched" || bad "sources touched without a seed"
has "...and says so" "target/ already present (not seeded)"

echo "A failed clone leaves no partial copy, and setup carries on unseeded:"
new_wt fails
STUB_CLONE_FAIL=1 setup
[[ -s "$WORK/cp.log" ]] && ok "the clone was attempted" || bad "no clone attempted"
[[ ! -e "$WT/target" ]] && ok "the partial target/ is removed" || bad "partial target/ left: $(ls -R "$WT/target")"
has "...says why and that the build is cold" "removed the partial copy, the first build here is cold"
has "...and setup goes on to the health check" "health check"
[[ "$(mtime "$WT/crates/a/src/lib.rs")" -lt 978307200 ]] && ok "...with no source touched" || bad "sources touched after a failed clone"

echo "No target/ in the primary: nothing to seed:"
mv "$MAIN/target" "$WORK/target.away"
new_wt empty
setup
[[ ! -s "$WORK/cp.log" && ! -e "$WT/target" ]] && ok "no clone and no target/" || bad "seeded from nothing: $(cat "$WORK/cp.log")"
has "...and says the first build is cold" "has no target/ to seed from"
mv "$WORK/target.away" "$MAIN/target"

echo ""
echo "setup-worktree: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
