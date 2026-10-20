#!/bin/bash
# ==============================================================================
# Tests for bin/wsl-clone.sh and bin/lib/wsl.sh: the WSL clone that runs the full
# gate off the Windows checkout. What create provisions, what remove refuses and
# what it removes, the C: free-space and MemAvailable preflight, the VHDX sparse
# check and the fstrim after removal, the build-env lines in the clone's .env,
# the Windows-interop node/pnpm refusal, and the marker checks on push and pr.
#
#   bash bin/lib/wsl-clone.test.sh
#
# Each case builds a scratch "main" checkout (on next, pushed to a bare origin)
# with the scripts under test copied into its bin/, because wsl-clone.sh finds its
# source repository from its own location. Stub reg.exe, fsutil.exe, df, wsl.exe,
# psql, pg_isready, pnpm, node, just and gh sit first on PATH. reg.exe, fsutil.exe and
# df serve, or derive from, bytes captured on a real WSL2 host (bin/lib/fixtures/wsl/,
# CRLF kept by its .gitattributes), as do the meminfo and /proc/version files. No real
# database, registry, disk or GitHub is touched.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/bin/wsl-clone.sh"
LIB="$ROOT/bin/lib/wsl.sh"
FIX="$ROOT/bin/lib/fixtures/wsl"
WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/wsl-clone-test.XXXXXX")" && pwd -P)"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR CARGO_TARGET_DIR
unset CARGO_BUILD_JOBS CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS OIS_WSL_CLONES OIS_WSL_SOURCE WSL_WINDOWS_MOUNT
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK" FIX

PASS=0
FAIL=0
SKIP=0
ok()   { echo "  PASS: $1"; PASS=$((PASS + 1)); }
skip() { echo "  SKIP: $1"; SKIP=$((SKIP + 1)); }
bad()  { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }
has()   { case "$OUT" in *"$2"*) ok "$1" ;; *) bad "$1 (no '$2' in: $OUT)" ;; esac; }
lacks() { case "$OUT" in *"$2"*) bad "$1 ('$2' in: $OUT)" ;; *) ok "$1" ;; esac; }

# Thresholds as literals from the requirement (GiB in KiB), never from wsl.sh.
C_FREE_OK_KB=20971520          # 20 GiB
C_FREE_SHORT_KB=20971519       # 20 GiB - 1 KiB
MEM_OK_KB=9437184              # 9 GiB
MEM_SHORT_KB=9437183           # 9 GiB - 1 KiB

# ------------------------------------------------------------------------------
# Stubs. All but pg_isready and node log their call to $STUB_LOG. Env knobs: STUB_REG
# (file), STUB_REG_RC, STUB_FSUTIL (file), STUB_FSUTIL_RC, STUB_DF (file), STUB_WSL_RC,
# STUB_JUST_RC, STUB_JUST_EDIT, STUB_PNPM_RC, STUB_PG_RC, STUB_PSQL_RC,
# STUB_CREATE_FAILS, STUB_DROP_FAILS, STUB_DB (file standing for the database),
# STUB_DB_NAME.
# ------------------------------------------------------------------------------
mkdir -p "$WORK/bin"
cat > "$WORK/bin/reg.exe" <<'STUB'
#!/bin/bash
echo "reg.exe $*" >> "$STUB_LOG"
[[ -n "${STUB_REG_RC:-}" ]] && exit "$STUB_REG_RC"
cat "${STUB_REG:-$FIX/reg-all.txt}"
STUB
cat > "$WORK/bin/fsutil.exe" <<'STUB'
#!/bin/bash
echo "fsutil.exe $*" >> "$STUB_LOG"
cat "${STUB_FSUTIL:-$FIX/fsutil-sparse.txt}"
exit "${STUB_FSUTIL_RC:-0}"
STUB
cat > "$WORK/bin/df" <<'STUB'
#!/bin/bash
echo "df $*" >> "$STUB_LOG"
cat "${STUB_DF:-$FIX/df.txt}"
STUB
cat > "$WORK/bin/wsl.exe" <<'STUB'
#!/bin/bash
echo "wsl.exe $*" >> "$STUB_LOG"
exit "${STUB_WSL_RC:-0}"
STUB
cat > "$WORK/bin/psql" <<'STUB'
#!/bin/bash
sql="${*: -1}"
echo "psql $sql" >> "$STUB_LOG"
echo "psql-url $1" >> "$STUB_LOG"
[[ -n "${STUB_PG_RC:-}" && "$STUB_PG_RC" != 0 ]] && exit 2
[[ -n "${STUB_PSQL_RC:-}" ]] && exit "$STUB_PSQL_RC"
case "$sql" in
    *"from pg_database"*"'$STUB_DB_NAME'"*) [[ -f "$STUB_DB" ]] && echo 1; exit 0 ;;
    *"create database"*"$STUB_DB_NAME"*) [[ -n "${STUB_CREATE_FAILS:-}" ]] && exit 1; touch "$STUB_DB"; exit 0 ;;
    *"drop database"*"$STUB_DB_NAME"*) [[ -n "${STUB_DROP_FAILS:-}" ]] && exit 1; rm -f "$STUB_DB"; exit 0 ;;
esac
exit 1
STUB
printf '#!/bin/bash\nexit "${STUB_PG_RC:-0}"\n' > "$WORK/bin/pg_isready"
printf '#!/bin/bash\necho "pnpm $* in $PWD" >> "$STUB_LOG"\nexit "${STUB_PNPM_RC:-0}"\n' > "$WORK/bin/pnpm"
printf '#!/bin/bash\nexit 0\n' > "$WORK/bin/node"
cat > "$WORK/bin/just" <<'STUB'
#!/bin/bash
echo "just $* in $PWD" >> "$STUB_LOG"
# STUB_JUST_EDIT=FILE: rewrite FILE in place mid-run (same inode, as an editor or a checkout
# writing over it does), shifting every byte the running bash has yet to read.
if [[ -n "${STUB_JUST_EDIT:-}" ]]; then
    { printf '# %s\n' "$(printf 'x%.0s' {1..4000})"; cat "$STUB_JUST_EDIT"; } > "$STUB_JUST_EDIT.new" \
        && cat "$STUB_JUST_EDIT.new" > "$STUB_JUST_EDIT" && rm -f "$STUB_JUST_EDIT.new"
fi
exit "${STUB_JUST_RC:-0}"
STUB
printf '#!/bin/bash\necho "gh $* in $PWD" >> "$STUB_LOG"\nexit 0\n' > "$WORK/bin/gh"
chmod +x "$WORK/bin/"*
export STUB_LOG="$WORK/stub.log"

# df_with_free KB OUT: the captured df output with line 2's Available field set to KB.
df_with_free() { awk -v kb="$1" 'NR == 2 { $4 = kb } { print }' "$FIX/df.txt" > "$2"; }
# meminfo_with_available KB OUT: the captured meminfo with MemAvailable set to KB.
meminfo_with_available() { sed -E "s/^(MemAvailable: +)[0-9]+ kB$/\\1$1 kB/" "$FIX/meminfo.txt" > "$2"; }
# not_wsl_version OUT: a native Linux /proc/version, no "microsoft" in it.
not_wsl_version() { printf 'Linux version 6.8.0-45-generic (buildd@lcy02-amd64-075) #45-Ubuntu SMP\n' > "$1"; }

# setup CASE: $MAIN (on next, pushed to a bare $ORIGIN) with the scripts under test
# copied into its bin/, a .env, and a local branch $BR = feat/1/<CASE> one commit
# ahead, checked out in a Windows-side worktree $WT. $NAME is the clone name
# (feat-1-<case>), $CLONE its directory under $OIS_WSL_CLONES, STUB_DB the file
# standing for its database ois_wsl_feat_1_<case>.
setup() {
    local base="$WORK/$1" f
    rm -rf "$base"; mkdir -p "$base"
    ORIGIN="$base/origin.git"
    git init -q --bare "$ORIGIN"
    MAIN="$base/main"
    git init -q -b next "$MAIN"
    mkdir -p "$MAIN/bin/lib" "$MAIN/.githooks" "$MAIN/.claude/hooks/lib"
    for f in bin/wsl-clone.sh bin/lib/wsl.sh bin/lib/portable.sh bin/check-marker.sh bin/lib/marker_manifest.py .claude/hooks/lib/attribution.sh; do
        [[ -f "$ROOT/$f" ]] && cp "$ROOT/$f" "$MAIN/$f"
    done
    printf '.env\n.claude/\nnode_modules/\n' > "$MAIN/.gitignore"
    printf 'lockfileVersion: 9.0\n' > "$MAIN/pnpm-lock.yaml"
    git -C "$MAIN" add -A && git -C "$MAIN" -c core.hooksPath=/dev/null commit -qm init
    git -C "$MAIN" remote add origin "$ORIGIN"
    git -C "$MAIN" push -q -u origin next
    printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\n' > "$MAIN/.env"
    BR="feat/1/$1"
    NAME="feat-1-$1"
    WT="$MAIN/.claude/worktrees/feat+1+$1"
    git -C "$MAIN" worktree add -q -b "$BR" "$WT" next
    printf 'x\n' > "$WT/file.txt" && git -C "$WT" add file.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -qm work
    export OIS_WSL_CLONES="$base/clones"
    CLONE="$OIS_WSL_CLONES/$NAME"
    STUB_DB="$base/db.exists"
    STUB_DB_NAME="ois_wsl_feat_1_$(printf '%s' "$1" | tr '-' '_')"
    export STUB_DB STUB_DB_NAME
    : > "$STUB_LOG"
}
# stamp_marker ROOT KIND SHA: a marker with the manifest line check_marker_for_head vouches for.
stamp_marker() { python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$@"; }
# wsl_clone [ARGS...]: runs $MAIN's copy of the script as on the captured WSL host; sets OUT
# (colors stripped) and RC. WSL_PROC_VERSION / WSL_MEMINFO / STUB_DF override per case.
wsl_clone() {
    OUT="$(cd "$WORK" && PATH="${TEST_PATH:-$WORK/bin:$PATH}" WSL_DISTRO_NAME="${WSL_DISTRO_NAME:-Ubuntu}" \
        WSL_PROC_VERSION="${WSL_PROC_VERSION:-$FIX/version.txt}" WSL_MEMINFO="${WSL_MEMINFO:-$FIX/meminfo.txt}" \
        bash "$MAIN/bin/wsl-clone.sh" "$@" 2>&1 | sed $'s/\033\\[[0-9;]*m//g'; exit "${PIPESTATUS[0]}")"
    RC=$?
}

echo "== harness: fixtures are the captured bytes, stubs serve them =="
for f in reg-all.txt fsutil-sparse.txt fsutil-notsparse.txt df.txt meminfo.txt version.txt; do
    [[ -s "$FIX/$f" ]] && ok "fixture $f is present" || bad "fixture $f missing"
done
[[ "$(cat "$FIX/.gitattributes")" == "* -text" ]] && ok ".gitattributes turns off EOL conversion" || bad ".gitattributes: $(cat "$FIX/.gitattributes" 2>&1)"
# Linux git cannot open a worktree Windows git registered (its .git names a C:/ path).
if ! git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    skip "git cannot open $ROOT from here, so its attributes are unchecked"
else
    [[ "$(git -C "$ROOT" check-attr text -- bin/lib/fixtures/wsl/reg-all.txt)" == *"text: unset" ]] && ok "...and git honors it" || bad "check-attr: $(git -C "$ROOT" check-attr text -- bin/lib/fixtures/wsl/reg-all.txt)"
fi
[[ "$(od -An -c "$FIX/fsutil-sparse.txt" | tr -s ' \n' ' ')" == " T h i s f i l e i s s e t a s s p a r s e \r \n " ]] && ok "fsutil-sparse.txt is the CRLF capture" || bad "fsutil-sparse.txt bytes: $(od -An -c "$FIX/fsutil-sparse.txt")"
[[ "$(head -c 31 "$FIX/fsutil-notsparse.txt")" == "This file is NOT set as sparse"$'\r' ]] && ok "fsutil-notsparse.txt is the CRLF capture" || bad "fsutil-notsparse.txt bytes"
# crs FILE / lfs FILE: count CR and LF bytes (grep's $ is CR-blind on some builds, so count bytes).
crs() { tr -cd '\r' < "$1" | wc -c | tr -d ' '; }
lfs() { tr -cd '\n' < "$1" | wc -c | tr -d ' '; }
[[ "$(crs "$FIX/reg-all.txt")" -eq "$(lfs "$FIX/reg-all.txt")" && "$(lfs "$FIX/reg-all.txt")" -gt 30 ]] && ok "every reg-all.txt line ends CRLF" || bad "reg-all.txt: $(crs "$FIX/reg-all.txt") CRs for $(lfs "$FIX/reg-all.txt") lines"
grep -q $'BasePath    REG_SZ    \\\\\\\\?\\\\C:' "$FIX/reg-all.txt" && ok "reg-all.txt carries a \\\\?\\ BasePath (docker-desktop)" || bad "reg-all.txt has no \\\\?\\ BasePath"
[[ "$(crs "$FIX/df.txt")$(crs "$FIX/meminfo.txt")$(crs "$FIX/version.txt")" == 000 ]] && ok "df, meminfo and version captures are LF, as Linux wrote them" || bad "a Linux capture gained a CR"
grep -qi microsoft "$FIX/version.txt" && ok "version.txt is a WSL kernel" || bad "version.txt lacks microsoft"
df_with_free "$C_FREE_SHORT_KB" "$WORK/df.short"
[[ "$(awk 'NR == 2 { print $4 }' "$WORK/df.short")" == 20971519 && "$(head -1 "$WORK/df.short")" == "$(head -1 "$FIX/df.txt")" ]] && ok "df_with_free sets only the Available field" || bad "df_with_free: $(cat "$WORK/df.short")"
meminfo_with_available "$MEM_SHORT_KB" "$WORK/meminfo.short"
[[ "$(grep '^MemAvailable:' "$WORK/meminfo.short")" == "MemAvailable:   9437183 kB" && "$(wc -l < "$WORK/meminfo.short")" -eq "$(wc -l < "$FIX/meminfo.txt")" ]] && ok "meminfo_with_available sets only MemAvailable" || bad "meminfo_with_available: $(grep '^MemAvailable' "$WORK/meminfo.short")"
: > "$STUB_LOG"
[[ "$(PATH="$WORK/bin:$PATH" reg.exe query x | grep -c 'DistributionName')" -eq 2 ]] && ok "the reg.exe stub serves reg-all.txt" || bad "reg.exe stub: $(PATH="$WORK/bin:$PATH" reg.exe query x)"
PATH="$WORK/bin:$PATH" fsutil.exe sparse queryflag y > "$WORK/fsutil.out"
cmp -s "$WORK/fsutil.out" "$FIX/fsutil-sparse.txt" && ok "the fsutil.exe stub serves the sparse capture byte for byte" || bad "fsutil.exe stub: $(od -c "$WORK/fsutil.out")"
[[ "$(cat "$STUB_LOG")" == "reg.exe query x"*"fsutil.exe sparse queryflag y" ]] && ok "...and each stub logs its call" || bad "stub log: $(cat "$STUB_LOG")"
setup harness
[[ "$(git -C "$MAIN" worktree list --porcelain | grep -c "^branch refs/heads/$BR$")" -eq 1 ]] && ok "setup builds a source repo with $BR in a worktree" || bad "setup: $(git -C "$MAIN" worktree list --porcelain)"

echo "== wsl.sh: names, constants, build env =="
lib() { bash -c '. "$1"; shift; "$@"' _ "$LIB" "$@"; }
[[ "$(lib wsl_clone_name feat/1/x-y)" == "feat-1-x-y" ]] && ok "wsl_clone_name writes / as -" || bad "wsl_clone_name: $(lib wsl_clone_name feat/1/x-y)"
[[ "$(lib wsl_db_name Feat-1.X)" == "ois_wsl_feat_1_x" ]] && ok "wsl_db_name lower-cases and folds to [a-z0-9_]" || bad "wsl_db_name: $(lib wsl_db_name Feat-1.X)"
LONG="$(lib wsl_db_name "$(printf 'a%.0s' {1..70})")"
[[ ${#LONG} -eq 63 && "$LONG" == ois_wsl_a* ]] && ok "...cut to 63 characters, prefix kept" || bad "long db name: $LONG (${#LONG})"
EMPTY="$(lib wsl_db_name "")"; rc=$?
[[ $rc -ne 0 && -z "$EMPTY" ]] && ok "wsl_db_name refuses an empty name rather than naming ois_wsl_" || bad "empty db name: rc $rc, '$EMPTY'"
CONSTS="$(bash -c '. "$1"; echo "$WSL_MIN_C_FREE_GB $WSL_MIN_MEM_AVAILABLE_GB"' _ "$LIB")"
[[ "$CONSTS" == "20 9" ]] && ok "the thresholds are 20 GiB of C: and 9 GiB of MemAvailable" || bad "thresholds: $CONSTS"
(( C_FREE_SHORT_KB < 20 * 1024 * 1024 && 20 * 1024 * 1024 <= C_FREE_OK_KB && MEM_SHORT_KB < 9 * 1024 * 1024 && 9 * 1024 * 1024 <= MEM_OK_KB )) \
    && ok "...and the boundary literals straddle them" || bad "boundary literals"
[[ "$(lib wsl_build_env_lines)" == $'CARGO_BUILD_JOBS=1\nCARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zthreads=1' ]] \
    && ok "wsl_build_env_lines prints exactly the two lines" || bad "build env: $(lib wsl_build_env_lines)"
[[ -z "$(bash -c '. "$1"' _ "$LIB" 2>&1)" ]] && ok "sourcing wsl.sh prints nothing" || bad "sourcing wsl.sh printed output"

echo "== AC2: preflight thresholds =="
[[ "$(PATH="$WORK/bin:$PATH" lib wsl_c_free_kb)" == 38548880 ]] && ok "C: free parses from the captured df" || bad "wsl_c_free_kb: $(PATH="$WORK/bin:$PATH" lib wsl_c_free_kb)"
[[ "$(WSL_MEMINFO="$FIX/meminfo.txt" lib wsl_mem_available_kb)" == 11023184 ]] && ok "MemAvailable parses from the captured meminfo" || bad "wsl_mem_available_kb"
df_with_free "$C_FREE_OK_KB" "$WORK/df.ok"
meminfo_with_available "$MEM_OK_KB" "$WORK/meminfo.ok"
setup preflight
STUB_DF="$WORK/df.short" WSL_MEMINFO="$WORK/meminfo.ok" wsl_clone preflight
[[ $RC -eq 1 ]] && has "C: free 1 KiB under 20 GiB is refused, naming C:" "C: free 19 GiB (20971519 KiB) is under 20 GiB" || bad "C: short: rc $RC: $OUT"
lacks "...with MemAvailable fine" "MemAvailable 8 GiB"
STUB_DF="$WORK/df.ok" WSL_MEMINFO="$WORK/meminfo.ok" wsl_clone preflight
[[ $RC -eq 0 ]] && ok "C: free of exactly 20 GiB and MemAvailable of exactly 9 GiB pass" || bad "at thresholds: rc $RC: $OUT"
has "...printing the C: figure and threshold" "C: free 20 GiB (minimum 20 GiB)"
has "...and the MemAvailable figure and threshold" "WSL MemAvailable 9 GiB (minimum 9 GiB)"
STUB_DF="$WORK/df.ok" WSL_MEMINFO="$WORK/meminfo.short" wsl_clone preflight
[[ $RC -eq 1 ]] && has "MemAvailable 1 KiB under 9 GiB is refused, naming it" "WSL MemAvailable 8 GiB (9437183 KiB) is under 9 GiB" || bad "mem short: rc $RC: $OUT"
grep -q '^df -Pk /mnt/c$' "$STUB_LOG" && ok "C: is read as df -Pk /mnt/c" || bad "df call: $(grep '^df' "$STUB_LOG")"
WSL_MEMINFO=/dev/null wsl_clone preflight
[[ $RC -eq 1 ]] && has "an unreadable MemAvailable is a failure, not a pass" "cannot read WSL MemAvailable" || bad "unreadable meminfo: rc $RC"
for short in df meminfo; do
    setup "create-short-$short"
    if [[ $short == df ]]; then export STUB_DF="$WORK/df.short" WSL_MEMINFO="$WORK/meminfo.ok"; else export STUB_DF="$WORK/df.ok" WSL_MEMINFO="$WORK/meminfo.short"; fi
    wsl_clone create "$BR"
    [[ $RC -ne 0 && ! -e "$CLONE" && ! -f "$STUB_DB" ]] && ok "create under the $short threshold creates no clone and no database" || bad "create short $short: rc $RC, clone $([[ -e $CLONE ]] && echo made)"
    grep -q '^pnpm' "$STUB_LOG" && bad "...but ran pnpm" || ok "...and runs no pnpm install"
    unset STUB_DF WSL_MEMINFO
    setup "gate-short-$short"
    wsl_clone create "$BR"
    : > "$STUB_LOG"
    if [[ $short == df ]]; then export STUB_DF="$WORK/df.short"; else export WSL_MEMINFO="$WORK/meminfo.short"; fi
    wsl_clone gate "$NAME"
    [[ $RC -ne 0 ]] && ok "gate under the $short threshold fails" || bad "gate short $short: rc $RC"
    grep -q '^just' "$STUB_LOG" && bad "...but ran just test" || ok "...without running just test"
    unset STUB_DF WSL_MEMINFO
done
setup gate
wsl_clone create "$BR"
printf 'y\n' > "$WT/two.txt" && git -C "$WT" add two.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -qm two
: > "$STUB_LOG"
wsl_clone gate "$BR"
[[ $RC -eq 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$(git -C "$WT" rev-parse HEAD)" ]] && ok "gate fast-forwards the clone to the Windows branch" || bad "gate ff: rc $RC: $OUT"
grep -q "^just test in .*$NAME$" "$STUB_LOG" && ok "...then runs just test in the clone" || bad "just call: $(cat "$STUB_LOG")"
STUB_JUST_RC=3 wsl_clone gate "$BR"
[[ $RC -eq 3 ]] && ok "gate exits with just test's status" || bad "gate rc $RC on a failing just test"
cp "$MAIN/bin/wsl-clone.sh" "$WORK/wsl-clone.orig"
STUB_JUST_EDIT="$MAIN/bin/wsl-clone.sh" wsl_clone gate "$BR"
[[ $RC -eq 0 ]] && lacks "a script rewritten during just test still exits with just test's status" "syntax error" || bad "rewritten mid-run: rc $RC: $OUT"
cp "$WORK/wsl-clone.orig" "$MAIN/bin/wsl-clone.sh"

echo "== AC1: create provisions the clone =="
setup create
printf '# dev\nDATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois?sslmode=disable\nRUST_LOG=info\n' > "$MAIN/.env"
wsl_clone create "$BR"
[[ $RC -eq 0 ]] && ok "create exits 0" || bad "create rc $RC: $OUT"
[[ "$(git -C "$CLONE" symbolic-ref --short HEAD)" == "$BR" && "$(git -C "$CLONE" rev-parse --abbrev-ref '@{u}')" == "windows/$BR" ]] \
    && ok "the clone has $BR checked out, tracking windows/$BR" || bad "clone branch: $(git -C "$CLONE" status -sb 2>&1 | head -1)"
[[ "$(git -C "$CLONE" rev-parse HEAD)" == "$(git -C "$WT" rev-parse HEAD)" ]] && ok "...at the Windows branch's commit" || bad "clone HEAD differs"
[[ "$(cd "$(git -C "$CLONE" remote get-url origin)" && pwd -P)" == "$(cd "$ORIGIN" && pwd -P)" ]] && ok "origin is the source's origin" || bad "origin: $(git -C "$CLONE" remote get-url origin)"
[[ -n "$(git -C "$CLONE" remote get-url windows)" && "$(git -C "$CLONE" rev-parse "windows/$BR")" == "$(git -C "$MAIN" rev-parse "$BR")" ]] && ok "windows is the source checkout" || bad "windows remote"
[[ "$(git -C "$CLONE" config core.hooksPath)" == ".githooks" ]] && ok "core.hooksPath is .githooks" || bad "hooksPath: $(git -C "$CLONE" config core.hooksPath)"
grep -qx "DATABASE_URL=\"postgres://ois:ois@127.0.0.1:5432/$STUB_DB_NAME?sslmode=disable\"" "$CLONE/.env" \
    && ok ".env points DATABASE_URL at the clone's database, query kept, quoted" || bad ".env: $(cat "$CLONE/.env")"
[[ "$(grep -c '^DATABASE_URL=' "$CLONE/.env")" -eq 1 ]] && grep -qx 'RUST_LOG=info' "$CLONE/.env" && grep -qx '# dev' "$CLONE/.env" \
    && ok "...one DATABASE_URL, every other line kept" || bad ".env lines: $(cat "$CLONE/.env")"
[[ -f "$STUB_DB" ]] && ok "the database $STUB_DB_NAME is created" || bad "no database: $(cat "$STUB_LOG")"
grep -q "^psql-url postgres://ois:ois@127.0.0.1:5432/ois?sslmode=disable$" "$STUB_LOG" && ok "...through the source's DATABASE_URL" || bad "admin url: $(grep psql-url "$STUB_LOG")"
grep -q "^pnpm install --frozen-lockfile.* in .*$NAME$" "$STUB_LOG" && ok "pnpm install --frozen-lockfile runs in the clone" || bad "pnpm: $(grep pnpm "$STUB_LOG")"
printf 'y\n' > "$WT/two.txt" && git -C "$WT" add two.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -qm two
: > "$STUB_LOG"
wsl_clone create "$BR"
[[ $RC -eq 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$(git -C "$WT" rev-parse HEAD)" ]] && ok "re-running create fast-forwards to the new commit" || bad "re-run: rc $RC: $OUT"
lacks "...by a fast-forward, not a reset" "was rewritten"
grep -q 'create database' "$STUB_LOG" && bad "...but issued create database for a database that exists" || ok "...and issues no create database for the existing one"
WINDOWS_URL="$(git -C "$CLONE" remote get-url windows)"
git -C "$CLONE" remote set-url windows "$WORK/no-such-source"
wsl_clone create "$BR"
[[ $RC -ne 0 ]] && has "create stops when the clone's windows remote cannot be fetched" "cannot fetch the clone's windows remote" || bad "windows fetch: rc $RC: $OUT"
git -C "$CLONE" remote set-url windows "$WINDOWS_URL"
printf 'z\n' > "$CLONE/local.txt" && git -C "$CLONE" add local.txt && git -C "$CLONE" -c core.hooksPath=/dev/null commit -qm local
LOCAL="$(git -C "$CLONE" rev-parse HEAD)"
wsl_clone create "$BR"
[[ $RC -ne 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$LOCAL" ]] && has "a clone with commits windows lacks is refused, HEAD kept" "the clone has commits windows/$BR lacks" || bad "local commits: rc $RC: $OUT"
setup unknown
wsl_clone create feat/1/no-such-branch
[[ $RC -ne 0 && ! -e "$OIS_WSL_CLONES/feat-1-no-such-branch" ]] && has "an unknown branch is refused, nothing cloned" "has no branch feat/1/no-such-branch" || bad "unknown: rc $RC: $OUT"
[[ ! -f "$STUB_DB" ]] && ok "...and no database made" || bad "unknown branch made a database"
setup not-wsl
not_wsl_version "$WORK/version.native"
wsl_clone create "$BR"
for sub in "create $BR" "gate $NAME" "push $NAME" "pr $NAME" "remove $NAME" preflight; do
    # shellcheck disable=SC2086
    WSL_PROC_VERSION="$WORK/version.native" wsl_clone $sub
    [[ $RC -ne 0 && "$OUT" == *"not inside WSL"* ]] && ok "$sub refuses outside WSL" || bad "$sub outside WSL: rc $RC: $OUT"
done
[[ -d "$CLONE" ]] && ok "...and remove outside WSL left the clone" || bad "remove outside WSL deleted the clone"
printf 'Linux version 4.4.0-19041-Microsoft (Microsoft@Microsoft.com)\n' > "$WORK/version.wsl1"
WSL_PROC_VERSION="$WORK/version.wsl1" wsl_clone preflight
[[ $RC -eq 0 ]] && ok "a capitalized Microsoft kernel string is accepted" || bad "WSL1 version refused: $OUT"

echo "== a copy of the script outside the checkout =="
setup copy
mkdir -p "$WORK/copy/bin/lib"
for f in bin/wsl-clone.sh bin/lib/wsl.sh bin/lib/portable.sh bin/lib/marker_manifest.py; do cp "$MAIN/$f" "$WORK/copy/$f"; done
# copy_clone [ARGS...]: wsl_clone, but running the copy outside any git checkout.
copy_clone() { local m="$MAIN"; MAIN="$WORK/copy"; wsl_clone "$@"; MAIN="$m"; }
copy_clone create "$BR"
[[ $RC -ne 0 && ! -e "$CLONE" ]] && has "a copy outside any checkout refuses, naming OIS_WSL_SOURCE" "set OIS_WSL_SOURCE to the primary checkout" || bad "copy without source: rc $RC: $OUT"
OIS_WSL_SOURCE="$WORK/copy" copy_clone create "$BR"
[[ $RC -ne 0 && ! -e "$CLONE" ]] && has "an OIS_WSL_SOURCE that is not a checkout is refused" "is not a git checkout" || bad "bad source: rc $RC: $OUT"
OIS_WSL_SOURCE="$MAIN" copy_clone create "$BR"
[[ $RC -eq 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$(git -C "$WT" rev-parse HEAD)" ]] && ok "with OIS_WSL_SOURCE the copy clones from that checkout" || bad "copy with source: rc $RC: $OUT"

echo "== a CRLF .env in the Windows checkout =="
setup crlf-env
printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\r\nRUST_LOG=info\r\n' > "$MAIN/.env"
wsl_clone create "$BR"
[[ $RC -eq 0 && -f "$STUB_DB" ]] && ok "a CRLF source .env still creates the database" || bad "crlf create: rc $RC: $OUT"
[[ "$(crs "$STUB_LOG")" -eq 0 ]] && ok "...and no CR reaches psql" || bad "a CR reached psql: $(grep psql-url "$STUB_LOG" | od -c | head -3)"
grep -q "^psql-url postgres://ois:ois@127.0.0.1:5432/ois$" "$STUB_LOG" && ok "...over a DATABASE_URL with no CR" || bad "crlf url: $(grep psql-url "$STUB_LOG" | od -c | head -3)"
[[ "$(crs "$CLONE/.env")" -eq 0 ]] && grep -qx 'RUST_LOG=info' "$CLONE/.env" && ok "the clone's .env has no CR, every line kept" || bad "crlf .env: $(od -c "$CLONE/.env" | head -5)"
touch "$STUB_DB"; git -C "$CLONE" push -q origin "$BR"; : > "$STUB_LOG"
wsl_clone remove "$NAME"
[[ $RC -eq 0 && ! -f "$STUB_DB" ]] && ok "remove drops the database through a CRLF source .env" || bad "crlf remove: rc $RC: $OUT"

echo "== AC4: build env in the clone's .env =="
setup buildenv
printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\nCARGO_BUILD_JOBS=8\nCARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zthreads=8\n' > "$MAIN/.env"
wsl_clone create "$BR"
[[ "$(grep -c '^CARGO_BUILD_JOBS=' "$CLONE/.env")" -eq 1 ]] && grep -qx 'CARGO_BUILD_JOBS=1' "$CLONE/.env" \
    && ok ".env sets CARGO_BUILD_JOBS=1 once, over the source's 8" || bad "jobs: $(grep CARGO "$CLONE/.env")"
[[ "$(grep -c '^CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=' "$CLONE/.env")" -eq 1 ]] \
    && grep -qx 'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zthreads=1' "$CLONE/.env" \
    && ok ".env sets the target rustflags to -Zthreads=1 once, over the source's" || bad "rustflags: $(grep CARGO "$CLONE/.env")"
grep -q 'Zthreads=1' "$ROOT/bin/lib/wsl.sh" && ok "wsl.sh holds the -Zthreads=1 value" || bad "wsl.sh lacks -Zthreads=1"
grep -qE 'Zthreads=1|CARGO_BUILD_JOBS=1' "$ROOT/bin/wsl-clone.sh" && bad "wsl-clone.sh writes a build env value of its own" || ok "...and wsl-clone.sh writes none of its own"

echo "== (a): node and pnpm must be Linux binaries =="
for tool in node pnpm; do
    setup "interop-$tool"
    mkdir -p "$WORK/mnt/c/bin"
    cp "$WORK/bin/$tool" "$WORK/mnt/c/bin/$tool"
    TEST_PATH="$WORK/mnt/c/bin:$WORK/bin:$PATH" WSL_WINDOWS_MOUNT="$WORK/mnt" wsl_clone create "$BR"
    [[ $RC -ne 0 && ! -e "$CLONE" ]] && has "$tool from the Windows mount is refused, nothing cloned" "$tool resolves to Windows's copy" || bad "interop $tool: rc $RC: $OUT"
    rm -rf "$WORK/mnt"
    # A PATH holding only the system tools the script needs, plus every stub but $tool.
    rm -rf "$WORK/only"; mkdir -p "$WORK/only"
    for f in "$WORK/bin/"*; do [[ "$(basename "$f")" == "$tool" ]] || cp "$f" "$WORK/only/"; done
    SYS=""
    for t in git bash sed awk grep tr cut mkdir dirname basename cat rm mv timeout env head tail od wc; do
        d="$(dirname "$(command -v "$t")")"; case ":$SYS:" in *":$d:"*) ;; *) SYS="$SYS:$d" ;; esac
    done
    if PATH="$WORK/only$SYS" command -v "$tool" >/dev/null 2>&1; then
        skip "a system $tool shares a directory with git here, so its absence cannot be staged"
    else
        TEST_PATH="$WORK/only$SYS" wsl_clone create "$BR"
        [[ $RC -ne 0 && ! -e "$CLONE" ]] && has "a missing $tool is refused, nothing cloned" "$tool not found in WSL" || bad "missing $tool: rc $RC: $OUT"
    fi
done

echo "== AC3: sparse VHDX check =="
setup sparse
wsl_clone create "$BR"
has "the sparse capture reports the disk sparse" 'ext4.vhdx is sparse'
grep -qF 'fsutil.exe sparse queryflag C:\Users\Matthew\AppData\Local\wsl\{bf97ba49-2f87-4e72-b74d-0094544fc778}\ext4.vhdx' "$STUB_LOG" \
    && ok "fsutil gets Ubuntu's BasePath and VhdFileName" || bad "fsutil call: $(grep fsutil "$STUB_LOG")"
[[ "$(crs "$STUB_LOG")" -eq 0 ]] && ok "...with the registry's CRs stripped" || bad "a CR reached a stub argument"
grep -qF 'reg.exe query HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss /s' "$STUB_LOG" && ok "reg.exe queries the Lxss key recursively" || bad "reg call: $(grep reg.exe "$STUB_LOG")"
sparse_warns() {
    has "$1 warns with wsl --shutdown" "wsl --shutdown"
    has "...and the set-sparse command" "wsl --manage ${2:-Ubuntu} --set-sparse true"
    [[ $RC -eq 0 ]] && ok "...and create still exits 0" || bad "$1: rc $RC"
}
STUB_FSUTIL="$FIX/fsutil-notsparse.txt" wsl_clone create "$BR"
sparse_warns "the not-sparse capture"
has "...naming the disk as not sparse" "ext4.vhdx is not sparse"
STUB_REG_RC=1 wsl_clone create "$BR"
sparse_warns "a failing reg.exe (as when interop is off)"
STUB_FSUTIL_RC=1 STUB_FSUTIL=/dev/null wsl_clone create "$BR"
sparse_warns "a failing fsutil.exe"
: > "$STUB_LOG"
WSL_DISTRO_NAME=NoSuchDistro wsl_clone create "$BR"
sparse_warns "a distro the registry does not list" NoSuchDistro
grep -q '^fsutil' "$STUB_LOG" && bad "...but fsutil was called" || ok "...without calling fsutil"
: > "$STUB_LOG"
WSL_DISTRO_NAME=docker-desktop wsl_clone create "$BR"
grep -qF 'fsutil.exe sparse queryflag C:\Users\Matthew\AppData\Local\Docker\wsl\main\ext4.vhdx' "$STUB_LOG" \
    && ok "a \\\\?\\ BasePath reaches fsutil with the prefix stripped" || bad "docker-desktop fsutil: $(grep fsutil "$STUB_LOG")"

echo "== AC1 + AC3: remove =="
# created_pushed CASE: a clone of a fresh setup, its branch pushed to origin.
created_pushed() { setup "$1"; wsl_clone create "$BR"; git -C "$CLONE" push -q origin "$BR"; : > "$STUB_LOG"; }
created_pushed remove
wsl_clone remove "$NAME"
[[ $RC -eq 0 && ! -e "$CLONE" ]] && ok "remove deletes a clean, pushed clone" || bad "remove: rc $RC: $OUT"
[[ ! -f "$STUB_DB" ]] && has "...drops its database" "database $STUB_DB_NAME dropped" || bad "database left: $(cat "$STUB_LOG")"
[[ "$(grep -c "from pg_database where datname = '$STUB_DB_NAME'" "$STUB_LOG")" -ge 2 ]] && ok "...and reads the drop back" || bad "no read-back: $(cat "$STUB_LOG")"
grep -q "^psql-url postgres://ois:ois@127.0.0.1:5432/ois$" "$STUB_LOG" && ok "...over the source's DATABASE_URL, not the database being dropped" || bad "drop url: $(grep psql-url "$STUB_LOG")"
[[ "$(tail -1 "$STUB_LOG")" == "wsl.exe -d Ubuntu -u root fstrim -v /" ]] && ok "...then runs fstrim as root, last" || bad "fstrim: $(cat "$STUB_LOG")"
git -C "$ORIGIN" show-ref --verify --quiet "refs/heads/$BR" && ok "...leaving the remote branch alone" || bad "remote branch deleted"
created_pushed remove-branch
wsl_clone remove "$BR"
[[ $RC -eq 0 && ! -e "$CLONE" ]] && ok "remove takes the branch name too" || bad "remove by branch: rc $RC: $OUT"
created_pushed fstrim-fails
STUB_WSL_RC=1 wsl_clone remove "$NAME"
[[ $RC -eq 1 && ! -e "$CLONE" && ! -f "$STUB_DB" ]] && ok "a failed fstrim exits 1 after the clone and database are gone" || bad "fstrim fail: rc $RC: $OUT"
has "...printing the command to run from Windows" "From Windows: wsl.exe -d Ubuntu -u root fstrim -v /"
created_pushed dirty
printf 'u\n' > "$CLONE/untracked.txt"
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" && -f "$STUB_DB" ]] && has "an untracked file blocks, clone and database kept" "has uncommitted or untracked changes" || bad "dirty: rc $RC: $OUT"
grep -q '^wsl.exe' "$STUB_LOG" && bad "...but fstrim ran" || ok "...and no fstrim"
created_pushed modified
printf 'm\n' >> "$CLONE/file.txt"
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && ok "a modified tracked file blocks" || bad "modified: rc $RC: $OUT"
setup unpushed
wsl_clone create "$BR"
wsl_clone remove "$NAME"
[[ $RC -eq 0 && ! -e "$CLONE" && ! -f "$STUB_DB" ]] && has "an unpushed clone that made no commits is removed: the Windows checkout holds its work" "came from windows/$BR" || bad "unpushed from windows: rc $RC: $OUT"
setup made-in-clone
wsl_clone create "$BR"
printf 'c\n' > "$CLONE/clone-only.txt" && git -C "$CLONE" add clone-only.txt && git -C "$CLONE" -c core.hooksPath=/dev/null commit -qm clone-only
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" && -f "$STUB_DB" ]] && has "a commit made in the clone, on neither origin/<branch> nor origin/next, blocks" "is neither on origin/$BR nor in origin/next" || bad "made in clone: rc $RC: $OUT"
git -C "$CLONE" push -q origin "$BR"
wsl_clone remove "$NAME"
[[ $RC -eq 0 && ! -e "$CLONE" ]] && has "a clone commit pushed to origin/<branch> is removed" "is on origin" || bad "pushed clone commit: rc $RC: $OUT"
setup made-in-clone-next
wsl_clone create "$BR"
printf 'c\n' > "$CLONE/clone-only.txt" && git -C "$CLONE" add clone-only.txt && git -C "$CLONE" -c core.hooksPath=/dev/null commit -qm clone-only
git -C "$CLONE" push -q origin "HEAD:refs/heads/next"
wsl_clone remove "$NAME"
[[ $RC -eq 0 && ! -e "$CLONE" ]] && ok "a clone commit already in origin/next is removed, its branch never pushed" || bad "on next: rc $RC: $OUT"
setup pruned
wsl_clone create "$BR"
printf 'c\n' > "$CLONE/clone-only.txt" && git -C "$CLONE" add clone-only.txt && git -C "$CLONE" -c core.hooksPath=/dev/null commit -qm clone-only
git -C "$CLONE" push -q origin "$BR"
git -C "$ORIGIN" update-ref -d "refs/heads/$BR"
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && ok "a clone commit whose branch was since deleted on origin is not taken as pushed" || bad "pruned: rc $RC: $OUT"
created_pushed unreachable
STUB_PG_RC=2 wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && has "an unreachable Postgres blocks, clone kept" "cannot reach Postgres" || bad "unreachable: rc $RC: $OUT"
grep -q '^wsl.exe' "$STUB_LOG" && bad "...but fstrim ran" || ok "...and no fstrim"
created_pushed drop-fails
STUB_DROP_FAILS=1 wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && has "a drop that does not take blocks, clone kept" "still exists after the drop" || bad "drop fails: rc $RC: $OUT"
setup gone
touch "$STUB_DB"
wsl_clone remove "$NAME"
[[ $RC -eq 0 && ! -f "$STUB_DB" ]] && has "with no clone, remove still drops the database" "no clone at" || bad "gone: rc $RC: $OUT"
[[ "$(tail -1 "$STUB_LOG")" == "wsl.exe -d Ubuntu -u root fstrim -v /" ]] && ok "...and runs fstrim" || bad "gone fstrim: $(cat "$STUB_LOG")"
setup bad-name
mkdir -p "$WORK/outside"; touch "$WORK/outside/keep"
mkdir -p "$OIS_WSL_CLONES"
for n in "../outside" "a b" "x;y" "." ".."; do
    wsl_clone remove "$n"
    [[ $RC -ne 0 ]] && has "remove refuses the name '$n' at the name guard" "is not a clone name or branch" || bad "name '$n': rc $RC: $OUT"
done
[[ -d "$OIS_WSL_CLONES" && -d "$OIS_WSL_CLONES/.." ]] && ok "...leaving the clones root and its parent" || bad "clones root removed"
wsl_clone remove ""
[[ $RC -eq 2 ]] && ok "an empty name prints usage" || bad "empty name: rc $RC"
[[ -f "$WORK/outside/keep" ]] && ok "...and nothing outside the clones root was removed" || bad "outside file removed"

echo "== push and pr: marker checks =="
setup push
wsl_clone create "$BR"
SHA="$(git -C "$CLONE" rev-parse HEAD)"
wsl_clone push "$NAME"
[[ $RC -ne 0 ]] && has "push without a tests marker is refused" "Tests (" || bad "push unmarked: rc $RC: $OUT"
git -C "$ORIGIN" show-ref --verify --quiet "refs/heads/$BR" && bad "...but the branch reached origin" || ok "...and nothing reaches origin"
mkdir -p "$CLONE/.claude/markers/tests" && touch "$CLONE/.claude/markers/tests/$SHA"
wsl_clone push "$NAME"
[[ $RC -ne 0 ]] && ok "a marker file with no manifest line is refused" || bad "unvouched marker pushed"
rm -rf "$CLONE/.claude/markers"
stamp_marker "$CLONE" tests "$SHA" >/dev/null
wsl_clone push "$NAME"
[[ $RC -eq 0 && "$(git -C "$ORIGIN" rev-parse "refs/heads/$BR" 2>/dev/null)" == "$SHA" ]] && ok "push with a vouched tests marker reaches origin" || bad "push: rc $RC: $OUT"
[[ "$(git -C "$CLONE" rev-parse --abbrev-ref '@{u}')" == "origin/$BR" ]] && ok "...and sets the upstream to origin" || bad "upstream: $(git -C "$CLONE" rev-parse --abbrev-ref '@{u}' 2>&1)"
printf 'p\n' > "$CLONE/later.txt" && git -C "$CLONE" add later.txt && git -C "$CLONE" -c core.hooksPath=/dev/null commit -qm later
stamp_marker "$CLONE" tests "$(git -C "$CLONE" rev-parse HEAD)" >/dev/null
# A git whose push reports success without sending anything.
mkdir -p "$WORK/fakepush"
REAL_GIT="$(command -v git)"
printf '#!/bin/bash\nfor a in "$@"; do [[ "$a" == push ]] && exit 0; done\nexec "%s" "$@"\n' "$REAL_GIT" > "$WORK/fakepush/git"
chmod +x "$WORK/fakepush/git"
TEST_PATH="$WORK/fakepush:$WORK/bin:$PATH" wsl_clone push "$NAME"
[[ $RC -ne 0 ]] && has "a push origin did not take is refused" "after the push" || bad "fake push: rc $RC: $OUT"
setup pr
wsl_clone create "$BR"
SHA="$(git -C "$CLONE" rev-parse HEAD)"
git -C "$CLONE" push -q origin "$BR"
: > "$STUB_LOG"
stamp_marker "$WT" review-shipping "$SHA" >/dev/null
wsl_clone pr "$NAME"
[[ $RC -ne 0 ]] && has "pr without the clone's tests marker is refused" "Tests (" || bad "pr no tests: rc $RC: $OUT"
stamp_marker "$CLONE" tests "$SHA" >/dev/null
rm -rf "$WT/.claude/markers"
wsl_clone pr "$NAME"
[[ $RC -ne 0 ]] && has "pr without the Windows worktree's review marker is refused" "Review (/review-before-shipping) has not run" || bad "pr no review: rc $RC: $OUT"
grep -q '^gh' "$STUB_LOG" && bad "...but gh ran" || ok "...and gh never runs"
stamp_marker "$WT" review-shipping "$SHA" >/dev/null
printf 'w\n' > "$WT/three.txt" && git -C "$WT" add three.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -qm three
stamp_marker "$WT" review-shipping "$(git -C "$WT" rev-parse HEAD)" >/dev/null
wsl_clone pr "$NAME"
[[ $RC -ne 0 ]] && has "pr refuses when the clone's HEAD is not the Windows branch's" "is not the Windows branch's" || bad "pr heads differ: rc $RC: $OUT"
grep -q '^gh' "$STUB_LOG" && bad "...but gh ran" || ok "...and gh never runs"
wsl_clone gate "$NAME"
SHA="$(git -C "$CLONE" rev-parse HEAD)"
stamp_marker "$CLONE" tests "$SHA" >/dev/null
: > "$STUB_LOG"
wsl_clone pr "$NAME" --title T
[[ $RC -ne 0 ]] && has "pr refuses while origin's branch is not the vouched commit" "is not ${SHA:0:8}" || bad "pr unpushed: rc $RC: $OUT"
git -C "$CLONE" push -q origin "$BR"
printf 'Body\r\n\r\nGenerated with [Claude Code](https://claude.com/claude-code)\r\n' > "$WORK/body-attr.md"
printf 'The client types are generated with openapi-typescript.\n' > "$WORK/body-prose.md"
for args in "--base main" "-B main" "--head other" "-Hother" "--repo x/y" "-R x/y" "--base=main" "--head=other" "--repo=x/y" "-dB main" "-dBmain" "-dH other"; do
    # shellcheck disable=SC2086
    wsl_clone pr "$NAME" $args
    [[ $RC -ne 0 ]] && has "pr refuses a caller's '$args'" "itself; drop it" || bad "pr '$args': rc $RC: $OUT"
done
for body in "Co-Authored-By: Claude Opus <noreply@anthropic.com>" "Session: https://claude.ai/code/session_01abc" "Drafted by Claude Code"; do
    wsl_clone pr "$NAME" --title T --body "$body"
    [[ $RC -ne 0 ]] && has "pr refuses an attribution line in --body ($body)" "carries an attribution line" || bad "pr body '$body': rc $RC: $OUT"
done
wsl_clone pr "$NAME" -t "Generated with [Claude Code]"
[[ $RC -ne 0 ]] && has "...and in a short flag's value (-t)" "carries an attribution line" || bad "pr -t attribution: rc $RC: $OUT"
for args in "-F -" "--body-file=-" "--body-file $WORK/no-such-body.md" "-F$WORK/no-such-body.md"; do
    # shellcheck disable=SC2086
    wsl_clone pr "$NAME" --title T $args
    [[ $RC -ne 0 ]] && has "pr refuses a body it cannot read ($args)" "attribution is unchecked" || bad "pr unreadable '$args': rc $RC: $OUT"
done
for args in "--editor" "-e" "--template t.md" "-T t.md" "--web" "-dw"; do
    # shellcheck disable=SC2086
    wsl_clone pr "$NAME" --title T $args
    [[ $RC -ne 0 ]] && has "pr refuses text from outside the checked arguments ($args)" "takes its text from" || bad "pr '$args': rc $RC: $OUT"
done
wsl_clone pr "$NAME" --title
[[ $RC -ne 0 ]] && has "pr refuses a last flag with no value" "has no value" || bad "pr dangling flag: rc $RC: $OUT"
mv "$MAIN/.claude/hooks/lib/attribution.sh" "$WORK/attribution.sh.away"
wsl_clone pr "$NAME" --title T --body "B"
[[ $RC -ne 0 ]] && has "pr refuses when the primary checkout's attribution library is missing" "cannot load" || bad "pr no library: rc $RC: $OUT"
mv "$WORK/attribution.sh.away" "$MAIN/.claude/hooks/lib/attribution.sh"
for form in "--body-file $WORK/body-attr.md" "-F $WORK/body-attr.md" "--body-file=$WORK/body-attr.md" "-F$WORK/body-attr.md"; do
    # shellcheck disable=SC2086
    wsl_clone pr "$NAME" --title T $form
    [[ $RC -ne 0 ]] && has "pr refuses an attribution line in a body file ($form)" "carries an attribution line" || bad "pr file '$form': rc $RC: $OUT"
done
grep -q '^gh' "$STUB_LOG" && bad "...but gh ran for a refused pr" || ok "...and gh never runs for any of them"
for args in "--body=Co-Authored-By: Claude <noreply@anthropic.com>" "-bCo-Authored-By: Claude <noreply@anthropic.com>"; do
    wsl_clone pr "$NAME" --title T "$args"
    [[ $RC -ne 0 ]] && has "pr refuses an attribution line attached to its flag (${args:0:7})" "carries an attribution line" || bad "pr attached '$args': rc $RC: $OUT"
done
printf 'Body\r\n\r\nDrafted by Claude Code\r\n' > "$WORK/body-drafted.md"
wsl_clone pr "$NAME" --title T -F "$WORK/body-drafted.md"
[[ $RC -ne 0 ]] && has "pr refuses a CRLF footer line" "carries an attribution line" || bad "pr crlf footer: rc $RC: $OUT"
: > "$STUB_LOG"
wsl_clone pr "$NAME" -t -dBmain --body x
[[ $RC -eq 0 ]] && grep -q -- "--title -dBmain\|-t -dBmain" "$STUB_LOG" && ok "a value that looks like flags (-t -dBmain) is a value, not a cluster" || bad "flag-like value: rc $RC: $OUT"
wsl_clone pr "$NAME" --title T -F "$WORK/body-prose.md"
[[ $RC -eq 0 ]] && ok "prose that names a tool generating code is not an attribution" || bad "pr prose body: rc $RC: $OUT"
wsl_clone pr "$NAME" --title T --body "B two"
[[ $RC -eq 0 ]] && ok "pr with both markers for the same HEAD runs" || bad "pr: rc $RC: $OUT"
grep -q "^gh pr create --repo VATUSA/OIS --base next --head $BR --title T --body B two in .*$NAME$" "$STUB_LOG" \
    && ok "...gh pr create in the clone, with the arguments passed through" || bad "gh call: $(cat "$STUB_LOG")"
git -C "$MAIN" worktree remove --force "$WT"
wsl_clone pr "$NAME"
[[ $RC -ne 0 ]] && has "pr refuses when no Windows worktree has the branch checked out" "no Windows worktree has $BR checked out" || bad "pr no worktree: rc $RC: $OUT"

echo "== main and next are never worked on =="
setup protected
printf 's\n' > "$MAIN/stray.txt" && git -C "$MAIN" add stray.txt && git -C "$MAIN" -c core.hooksPath=/dev/null commit -qm stray
NEXT_ON_ORIGIN="$(git -C "$ORIGIN" rev-parse refs/heads/next)"
for b in next main; do
    wsl_clone create "$b"
    [[ $RC -ne 0 && ! -e "$OIS_WSL_CLONES/$b" ]] && has "create refuses $b, nothing cloned" "$b is a protected branch" || bad "create $b: rc $RC: $OUT"
done
# A clone that is on next anyway (made by hand, or left by an older script).
git clone -q --origin windows "$MAIN" "$OIS_WSL_CLONES/next"
git -C "$OIS_WSL_CLONES/next" remote add origin "$ORIGIN"
SHA="$(git -C "$OIS_WSL_CLONES/next" rev-parse HEAD)"
stamp_marker "$OIS_WSL_CLONES/next" tests "$SHA" >/dev/null
: > "$STUB_LOG"
for sub in gate push pr; do
    wsl_clone "$sub" next
    [[ $RC -ne 0 ]] && has "$sub refuses a clone on next" "next is a protected branch" || bad "$sub next: rc $RC: $OUT"
done
[[ "$(git -C "$ORIGIN" rev-parse refs/heads/next)" == "$NEXT_ON_ORIGIN" ]] && ok "...and origin's next did not move" || bad "origin next moved"
grep -qE '^(just|gh)' "$STUB_LOG" && bad "...but just or gh ran" || ok "...and neither just nor gh ran"
setup other-branch
wsl_clone create "$BR"
git -C "$CLONE" checkout -q -b feat/1/something-else
wsl_clone create "$BR"
[[ $RC -ne 0 ]] && has "create refuses a clone with another branch checked out" "has feat/1/something-else checked out, not $BR" || bad "other branch: rc $RC: $OUT"

echo "== failures that must stop the script =="
setup no-git
mkdir -p "$CLONE"
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && has "remove refuses a directory git cannot read" "git status failed" || bad "no-git remove: rc $RC: $OUT"
setup no-path-url
printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432\n' > "$MAIN/.env"
wsl_clone create "$BR"
grep -qx "DATABASE_URL=\"postgres://ois:ois@127.0.0.1:5432/$STUB_DB_NAME\"" "$CLONE/.env" && ok "a DATABASE_URL with no database gets the clone's appended" || bad "no-path url: $(grep DATABASE_URL "$CLONE/.env")"
setup no-db-url
printf 'RUST_LOG=info\n' > "$MAIN/.env"
wsl_clone create "$BR"
[[ $RC -ne 0 && ! -f "$STUB_DB" ]] && has "a source .env without DATABASE_URL stops create" "sets no DATABASE_URL" || bad "no db url: rc $RC: $OUT"
[[ ! -e "$CLONE/.env" ]] && ok "...before it writes the clone's .env" || bad "a .env was written without DATABASE_URL: $(cat "$CLONE/.env")"
setup pnpm-fails
STUB_PNPM_RC=1 wsl_clone create "$BR"
[[ $RC -ne 0 ]] && has "a failed pnpm install stops create" "pnpm install --frozen-lockfile failed" || bad "pnpm fails: rc $RC: $OUT"
lacks "...before it reports the clone ready" "Clone ready"
STUB_DF=/dev/null wsl_clone preflight
[[ $RC -ne 0 ]] && has "an unreadable C: figure is a failure, not a pass" "cannot read C: free space" || bad "unreadable df: rc $RC"
wsl_clone gate feat-1-never-made
[[ $RC -ne 0 ]] && has "gate with no clone says how to make one" "no clone at" || bad "gate no clone: rc $RC"
wsl_clone frobnicate
[[ $RC -eq 2 ]] && has "an unknown subcommand prints usage" "bash bin/wsl-clone.sh create <branch>" || bad "unknown subcommand: rc $RC: $OUT"
setup crlf-portable
tr -d '\r' < "$MAIN/bin/lib/portable.sh" | sed 's/$/\r/' > "$WORK/portable.crlf" && cp "$WORK/portable.crlf" "$MAIN/bin/lib/portable.sh"
[[ "$(crs "$MAIN/bin/lib/portable.sh")" -gt 10 ]] || bad "could not stage a CRLF portable.sh"
wsl_clone create "$BR"
[[ $RC -eq 0 && -f "$STUB_DB" ]] && ok "a CRLF portable.sh (a Windows checkout) still loads" || bad "crlf portable: rc $RC: $OUT"

setup quoted-url
printf 'DATABASE_URL="postgres://ois:ois@127.0.0.1:5432/first"   # dev\nDATABASE_URL=postgres://ois:ois@127.0.0.1:5432/stale\nDATABASE_URL="postgres://ois:ois@127.0.0.1:5432/ois"  # last wins\n' > "$MAIN/.env"
wsl_clone create "$BR"
[[ "$(grep '^psql-url' "$STUB_LOG" | sort -u)" == "psql-url postgres://ois:ois@127.0.0.1:5432/ois" ]] \
    && ok "the source DATABASE_URL is read unquoted, its comment dropped, the last line winning" || bad "quoted url: $(grep psql-url "$STUB_LOG" | sort -u)"
grep -qx "DATABASE_URL=\"postgres://ois:ois@127.0.0.1:5432/$STUB_DB_NAME\"" "$CLONE/.env" && ok "...and the clone's URL is built from that last line" || bad "clone url: $(grep DATABASE_URL "$CLONE/.env")"
setup psql-fails
STUB_PSQL_RC=1 wsl_clone create "$BR"
[[ $RC -ne 0 && ! -f "$STUB_DB" ]] && has "a failing psql stops create" "cannot query Postgres" || bad "psql fails: rc $RC: $OUT"
setup create-fails
STUB_CREATE_FAILS=1 wsl_clone create "$BR"
[[ $RC -ne 0 ]] && has "a create that did not take stops create" "does not exist after create" || bad "create fails: rc $RC: $OUT"
setup no-vhdfile
grep -v 'VhdFileName' "$FIX/reg-all.txt" > "$WORK/reg-novhd.txt"
: > "$STUB_LOG"
STUB_REG="$WORK/reg-novhd.txt" wsl_clone create "$BR"
grep -qF 'fsutil.exe sparse queryflag C:\Users\Matthew\AppData\Local\wsl\{bf97ba49-2f87-4e72-b74d-0094544fc778}\ext4.vhdx' "$STUB_LOG" \
    && ok "a distro with no VhdFileName defaults to ext4.vhdx" || bad "no VhdFileName: $(grep fsutil "$STUB_LOG")"
setup fetch-fails
wsl_clone create "$BR"
git -C "$CLONE" push -q origin "$BR"
git -C "$CLONE" remote set-url origin "$WORK/no-such-origin.git"
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && has "remove refuses when origin cannot be fetched" "cannot fetch origin" || bad "fetch fails: rc $RC: $OUT"
setup detached
wsl_clone create "$BR"
git -C "$CLONE" checkout -q --detach
wsl_clone push "$NAME"
[[ $RC -ne 0 ]] && has "push refuses a detached clone" "is not on a branch" || bad "detached: rc $RC: $OUT"

echo "== Windows worktree paths as Linux git prints them =="
WT_LINE="$(awk '/^worktree / { wt = substr($0, 10) } $1 == "branch" && $2 == "refs/heads/chore/872/wsl-gate-clone" { print wt }' "$FIX/worktree-list.txt")"
[[ "$WT_LINE" == */C:/Users/* ]] && ok "the captured listing prints the Windows worktree under the common dir" || bad "fixture line: $WT_LINE"
[[ "$(lib wsl_mount_path "$WT_LINE")" == "/mnt/c/Users/Matthew/orca/workspaces/OIS/ticket-loops-wsl-gate-clone-is-unmanaged-target" ]] \
    && ok "wsl_mount_path reads its drive path through /mnt" || bad "wsl_mount_path: $(lib wsl_mount_path "$WT_LINE")"
[[ "$(lib wsl_mount_path 'C:\Users\M\wt')" == "/mnt/c/Users/M/wt" ]] && ok "...a backslash path too" || bad "backslash path: $(lib wsl_mount_path 'C:\Users\M\wt')"
[[ "$(lib wsl_mount_path "$WORK")" == "$WORK" ]] && ok "an existing directory passes through unchanged" || bad "existing dir changed"

echo "== remaining refusals =="
[[ "$(env -u WSL_WINDOWS_MOUNT bash -c '. "$1"; echo "$WSL_WINDOWS_MOUNT"' _ "$LIB")" == "/mnt" ]] && ok "the Windows mount defaults to /mnt" || bad "mount default"
wsl_clone create
[[ $RC -eq 2 ]] && ok "create with no branch prints usage" || bad "create no branch: rc $RC"
setup remove-no-url
wsl_clone create "$BR"
git -C "$CLONE" push -q origin "$BR"
printf 'RUST_LOG=info\n' > "$MAIN/.env"
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && has "remove refuses without a source DATABASE_URL, clone kept" "sets no DATABASE_URL" || bad "remove no url: rc $RC: $OUT"

echo "== the Windows branch rewritten before its first push =="
# rewrite KIND: amend, squash or rebase the Windows branch $BR after the clone took its tip.
rewrite() {
    case "$1" in
        amend) printf 'amended\n' > "$WT/file.txt" && git -C "$WT" add file.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -q --amend -m work-amended ;;
        squash)
            printf 's\n' > "$WT/second.txt" && git -C "$WT" add second.txt && git -C "$WT" -c core.hooksPath=/dev/null commit -qm second
            wsl_clone create "$BR"
            git -C "$WT" reset -q --soft next && git -C "$WT" -c core.hooksPath=/dev/null commit -qm squashed ;;
        rebase)
            printf 'n\n' > "$MAIN/on-next.txt" && git -C "$MAIN" add on-next.txt && git -C "$MAIN" -c core.hooksPath=/dev/null commit -qm on-next
            git -C "$MAIN" push -q origin next
            git -C "$WT" -c core.hooksPath=/dev/null rebase -q next ;;
    esac
}
for kind in amend squash rebase; do
    setup "rewrite-$kind"
    wsl_clone create "$BR"
    OLD_HEAD="$(git -C "$CLONE" rev-parse HEAD)"
    rewrite "$kind"
    [[ "$(git -C "$WT" rev-parse HEAD)" != "$OLD_HEAD" ]] && ! git -C "$WT" merge-base --is-ancestor "$OLD_HEAD" HEAD \
        || bad "$kind did not rewrite the branch (fixture)"
    : > "$STUB_LOG"
    wsl_clone gate "$BR"
    [[ $RC -eq 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$(git -C "$WT" rev-parse HEAD)" ]] \
        && has "after a $kind, gate moves the clone to the rewritten tip" "was rewritten" || bad "$kind gate: rc $RC: $OUT"
    grep -q "^just test in .*$NAME$" "$STUB_LOG" && ok "...and runs just test in the clone" || bad "$kind gate: no just test in the clone: $(grep just "$STUB_LOG")"
    rewrite amend
    wsl_clone remove "$NAME"
    [[ $RC -eq 0 && ! -e "$CLONE" && ! -f "$STUB_DB" ]] && ok "after a $kind, remove takes the clone and its database" || bad "$kind remove: rc $RC: $OUT"
done
setup rewrite-then-local
wsl_clone create "$BR"
rewrite amend
printf 'c\n' > "$CLONE/clone-only.txt" && git -C "$CLONE" add clone-only.txt && git -C "$CLONE" -c core.hooksPath=/dev/null commit -qm clone-only
LOCAL="$(git -C "$CLONE" rev-parse HEAD)"
wsl_clone gate "$BR"
[[ $RC -ne 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$LOCAL" ]] && has "a commit made in the clone is still refused after a rewrite, HEAD kept" "the clone has commits windows/$BR lacks" || bad "rewrite then local: rc $RC: $OUT"
wsl_clone remove "$NAME"
[[ $RC -ne 0 && -d "$CLONE" ]] && ok "...and remove keeps it" || bad "rewrite then local remove: rc $RC: $OUT"
setup rewrite-dirty
wsl_clone create "$BR"
rewrite amend
printf 'd\n' >> "$CLONE/file.txt"
wsl_clone gate "$BR"
[[ $RC -ne 0 ]] && grep -q '^d$' "$CLONE/file.txt" && has "a dirty clone is not reset onto a rewritten branch" "uncommitted or untracked changes" || bad "rewrite dirty: rc $RC: $OUT"

echo "== a first fetch that fails =="
setup fetch-first-fails
mkdir -p "$WORK/nofetch"
REAL_GIT="$(command -v git)"
printf '#!/bin/bash\nfor a in "$@"; do [[ "$a" == fetch ]] && exit 1; done\nexec "%s" "$@"\n' "$REAL_GIT" > "$WORK/nofetch/git"
chmod +x "$WORK/nofetch/git"
PARTIAL="$OIS_WSL_CLONES/.$NAME.partial"
TEST_PATH="$WORK/nofetch:$WORK/bin:$PATH" wsl_clone create "$BR"
[[ $RC -ne 0 && ! -e "$CLONE" ]] && has "a failed first fetch leaves nothing at the clone path" "run create again" || bad "first fetch fails: rc $RC, clone $([[ -e $CLONE ]] && echo left): $OUT"
[[ -d "$PARTIAL/.git" ]] && ok "...only the partial clone beside it" || bad "no partial clone left to clear"
wsl_clone create "$BR"
[[ $RC -eq 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$(git -C "$WT" rev-parse HEAD)" ]] && ok "...so create succeeds on retry" || bad "retry after failed fetch: rc $RC: $OUT"
[[ ! -e "$PARTIAL" ]] && ok "...and the partial clone is gone" || bad "partial clone left after a retry"

echo "== an interrupted build =="
setup interrupted
PARTIAL="$OIS_WSL_CLONES/.$NAME.partial"
# What a build killed mid-fetch leaves: an initialized repo with an unborn HEAD, plus junk.
mkdir -p "$OIS_WSL_CLONES" && git init -q "$PARTIAL" && touch "$PARTIAL/junk"
wsl_clone create "$BR"
[[ $RC -eq 0 && "$(git -C "$CLONE" rev-parse HEAD)" == "$(git -C "$WT" rev-parse HEAD)" ]] && ok "create succeeds over a partial clone an interrupted build left" || bad "after interrupt: rc $RC: $OUT"
[[ ! -e "$PARTIAL" && ! -e "$CLONE/junk" ]] && ok "...clearing it first" || bad "partial left or reused"
git -C "$CLONE" push -q origin "$BR"
git init -q "$PARTIAL"
wsl_clone remove "$NAME"
[[ $RC -eq 0 && ! -e "$CLONE" && ! -e "$PARTIAL" ]] && ok "remove takes a leftover partial clone too" || bad "remove partial: rc $RC: $OUT"

echo "== a non-clone directory at the clone path =="
setup foreign-dir
mkdir -p "$CLONE" && printf 'mine\n' > "$CLONE/keep.txt"
wsl_clone create "$BR"
[[ $RC -ne 0 ]] && has "create refuses a directory that is not a clone" "exists and is not a clone" || bad "foreign dir: rc $RC: $OUT"
[[ -f "$CLONE/keep.txt" && ! -e "$CLONE/.git" ]] && ok "...and leaves its contents alone" || bad "foreign dir contents changed"

echo
echo "wsl-clone: $PASS passed, $FAIL failed, $SKIP skipped"
[ "$FAIL" -eq 0 ]
