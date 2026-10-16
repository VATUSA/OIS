#!/bin/bash
# ==============================================================================
# Tests for bin/review-scan.sh.
#
#   bash bin/lib/review-scan.test.sh
#
# Builds a miniature of OIS's layout with origin/next faked by update-ref: a base
# commit, a branch that keeps every sync invariant (must report no layer-1 finding)
# and a branch that breaks each one (must report each). Then a branch that adds one
# file per layer-2 pattern check, read by which file:line each message follows. The
# script, the shell tests' fixtures and the guidance under .claude/ name the patterns
# and are left out of layer 2. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCAN="$ROOT/bin/review-scan.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/review-scan-test.XXXXXX")"
trap 'chmod -R u+rw "$WORK" 2>/dev/null; rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REPO="$WORK/repo"
g() { git -C "$REPO" -c core.hooksPath=/dev/null "$@"; }
git init -q -b next "$REPO"
mkdir -p "$REPO"/backend/src/auth "$REPO"/backend/src/repos "$REPO"/backend/src/handlers "$REPO"/backend/migrations "$REPO"/crates/ois-core/src
cat >"$REPO/backend/src/auth/permissions.rs" <<'RS'
permission!(TmuTmiRead, ["tmu", "tmi"], Read);
permission!(TmuTmiUpdate, ["tmu", "tmi"], Update);
RS
cat >"$REPO/crates/ois-core/src/catalog.rs" <<'RS'
pub fn default_roles() -> Vec<&'static str> {
    vec![
        SERVER_ADMIN_ROLE,
        "USER",
    ]
}

pub fn draft_new_permission_names() -> Vec<&'static str> {
    vec![
        "tmu.tmi.read",
        "tmu.tmi.update",
    ]
}
RS
cat >"$REPO/backend/src/repos/access.rs" <<'RS'
pub const ASSIGNABLE_USER_ROLES: &[&str] = &[
    "USER",
];
RS
cat >"$REPO/backend/migrations/0001_init.sql" <<'SQL'
insert into access.permissions (name, description) values
    ('tmu.tmi.read', 'Read TMIs'),
    ('tmu.tmi.update', 'Update TMIs');
insert into access.roles (name, description) values
    ('USER', 'Everyone');
SQL
cat >"$REPO/backend/src/router.rs" <<'RS'
use crate::handlers::{jobs as jobs_handler, tmu};
pub fn build_router() -> Router {
    Router::new()
        .route("/api/v1/tmu/tmis", get(tmu::list_tmis))
        .route("/api/v1/tmu/tmis/{id}", patch(tmu::update_tmi))
}
RS
cat >"$REPO/backend/src/openapi.rs" <<'RS'
#[openapi(
    paths(
        crate::handlers::tmu::list_tmis,
        crate::handlers::tmu::update_tmi,
    )
)]
pub struct ApiDoc;
RS
cat >"$REPO/backend/src/handlers/tmu.rs" <<'RS'
pub async fn list_tmis(
    _permission: RequirePermission<TmuTmiRead>,
) -> Result<Json<Vec<Tmi>>, ApiError> {
    tmu_repo::list().await
}

pub async fn update_tmi(
    _permission: RequirePermission<TmuTmiUpdate>,
    Path(id): Path<String>,
) -> Result<Json<Tmi>, ApiError> {
    tmu_repo::update(&id).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}
RS
g add -A && g commit -qm base
g update-ref refs/remotes/origin/next HEAD

# run_scan [ARGS...]: sets OUT and RC.
run_scan() { OUT="$(cd "$REPO" && bash "$SCAN" "$@" 2>&1)"; RC=$?; }

echo "Layer 1, a branch that keeps every invariant:"
# A fully wired permission + route, an aliased router module, and test-only SQL/unwrap.
g checkout -q -b clean
perl -pi -e 'print "permission!(TmuTmiCreate, [\"tmu\", \"tmi\"], Create);\n" if $. == 1' "$REPO/backend/src/auth/permissions.rs"
perl -pi -e 's/^(\s+)"tmu.tmi.read",/$1"tmu.tmi.read",\n$1"tmu.tmi.create",/' "$REPO/crates/ois-core/src/catalog.rs"
printf "insert into access.permissions (name, description) values\n    ('tmu.tmi.create', 'Create TMIs');\n" >"$REPO/backend/migrations/0002_tmi_create.sql"
perl -pi -e 's/^(\s+)\.route\("\/api\/v1\/tmu\/tmis", get\(tmu::list_tmis\)\)/$1.route("\/api\/v1\/tmu\/tmis", get(tmu::list_tmis).post(tmu::create_tmi))\n$1.route("\/api\/v1\/jobs", get(jobs_handler::list_jobs))/' "$REPO/backend/src/router.rs"
perl -pi -e 's/^(\s+)crate::handlers::tmu::list_tmis,/$1crate::handlers::tmu::list_tmis,\n$1crate::handlers::tmu::create_tmi,\n$1crate::handlers::jobs::list_jobs,/' "$REPO/backend/src/openapi.rs"
perl -0pi -e 's/#\[cfg\(test\)\]/pub async fn create_tmi(\n    _permission: RequirePermission<TmuTmiCreate>,\n    Query(query): Query<CreateQuery>,\n) -> Result<Json<Tmi>, ApiError> {\n    tmu_repo::create().await\n}\n\nasync fn reload(pool: &sqlx::PgPool) -> Result<(), ApiError> {\n    tmu_repo::load(pool).await\n}\n\n#[cfg(test)]/; s/fn t\(\) \{\}/fn t() {\n        let n: i64 = sqlx::query_scalar("select 1").fetch_one(p).await.unwrap();\n    }/' "$REPO/backend/src/handlers/tmu.rs"
g add -A && g commit -qm clean
run_scan
[[ $RC -eq 0 ]] && ok "the scan exits 0" || bad "clean branch exit (want 0, got $RC): $OUT"
if grep -qE '^[^ ]+:[0-9]+: ' <<<"$OUT"; then bad "no layer-1 finding on a clean branch ($OUT)"; else ok "no layer-1 finding on a clean branch"; fi
grep -q '^review-scan: no sync-invariant finding vs origin/next$' <<<"$OUT" && ok "...and says so, against the default base origin/next" || bad "clean summary line ($OUT)"
[[ "$(tail -1 <<<"$OUT")" == "scan covered 6 file(s)" ]] && ok "the last line counts the 6 changed files" || bad "coverage line (want 6, got: $(tail -1 <<<"$OUT"))"

echo "Layer 1, a branch that breaks each invariant:"
g checkout -q -b dirty next
perl -pi -e 'print "permission!(FlowFooUpdate, [\"flow\", \"foo\"], Update);\n" if $. == 1' "$REPO/backend/src/auth/permissions.rs"
perl -pi -e 's/^(\s+)"tmu.tmi.read",/$1"tmu.tmi.read",\n$1"flow.bar.read",/; s/^(\s+)"USER",/$1"USER",\n$1"OTHER_ROLE",/' "$REPO/crates/ois-core/src/catalog.rs"
perl -pi -e 's/^(\s+)"USER",/$1"USER",\n$1"THIRD_ROLE",/' "$REPO/backend/src/repos/access.rs"
printf "insert into access.permissions (name, description) values\n    ('flow.baz.read', 'x');\ninsert into access.roles (name, description) values\n    ('NEW_ROLE', 'x');\n" >"$REPO/backend/migrations/0002_flow.sql"
echo "-- edited after it shipped" >>"$REPO/backend/migrations/0001_init.sql"
perl -pi -e 's/^(\s+)\.route\("\/api\/v1\/tmu\/tmis", get\(tmu::list_tmis\)\)/$1.route("\/api\/v1\/tmu\/tmis", get(tmu::list_tmis))\n$1.route("\/api\/v1\/tmu\/purge", delete(tmu::purge_tmis))/' "$REPO/backend/src/router.rs"
perl -pi -e 's/^(\s+)crate::handlers::tmu::list_tmis,/$1crate::handlers::tmu::list_tmis,\n$1crate::handlers::tmu::orphan_handler,/' "$REPO/backend/src/openapi.rs"
perl -0pi -e 's/    _permission: RequirePermission<TmuTmiUpdate>,\n//; s/#\[cfg\(test\)\]/pub async fn purge_tmis(State(state): State<AppState>) -> Result<Json<()>, ApiError> {\n    let rows = sqlx::query("delete from tmu.tmis").execute(&state.db).await;\n    let n = rows.unwrap();\n    Ok(Json(()))\n}\n\n#[cfg(test)]/' "$REPO/backend/src/handlers/tmu.rs"
g add -A && g commit -qm dirty
run_scan
[[ $RC -eq 0 ]] && ok "the scan exits 0 with findings (they are cues)" || bad "dirty branch exit (want 0, got $RC): $OUT"
# finding NAME REGEX: a layer-1 line matching REGEX
finding() { if grep -qE "$2" <<<"$OUT"; then ok "$1"; else bad "$1 (no /$2/ in: $OUT)"; fi; }
finding "marker without a catalog string"     '^backend/src/auth/permissions.rs:1: permission flow.foo.update has a marker but no "flow.foo.update" in'
finding "marker without a migration row"      '^backend/src/auth/permissions.rs:1: permission flow.foo.update has a marker but no access.permissions row'
finding "catalog permission without a row"    '^crates/ois-core/src/catalog.rs:[0-9]+: permission flow.bar.read is in the catalog but no migration'
finding "default role without a row"          '^crates/ois-core/src/catalog.rs:[0-9]+: role OTHER_ROLE is in default_roles\(\) but no migration'
finding "assignable role not in the catalog"  '^backend/src/repos/access.rs:3: role THIRD_ROLE is assignable but missing from default_roles'
finding "assignable role without a row"       '^backend/src/repos/access.rs:3: role THIRD_ROLE is assignable but no migration'
finding "inserted permission not in catalog"  '^backend/migrations/0002_flow.sql:2: permission flow.baz.read is inserted but missing'
finding "inserted role not in default_roles"  '^backend/migrations/0002_flow.sql:4: role NEW_ROLE is inserted but missing from default_roles'
finding "inserted role not assignable"        '^backend/migrations/0002_flow.sql:4: role NEW_ROLE is not in ASSIGNABLE_USER_ROLES'
finding "applied migration edited"            '^backend/migrations/0001_init.sql:1: applied migration edited in place'
finding "route missing from openapi.rs"       '^backend/src/router.rs:[0-9]+: route tmu::purge_tmis is not registered in backend/src/openapi.rs'
finding "mutating route without a permission" '^backend/src/handlers/tmu.rs:[0-9]+: mutating route \(delete\) tmu::purge_tmis takes no RequirePermission'
finding "openapi path with no route"          '^backend/src/openapi.rs:[0-9]+: path tmu::orphan_handler is in the OpenAPI document but no route'
finding "SQL in a handler"                    '^backend/src/handlers/tmu.rs:[0-9]+: SQL in a handler'
finding "unwrap in a handler"                 '^backend/src/handlers/tmu.rs:[0-9]+: unwrap\(\)/expect\(\) in a handler'
finding "a RequirePermission removed"         '^backend/src/handlers/tmu.rs:8: RequirePermission<TmuTmiUpdate> was removed'
finding "the summary counts them"             '^review-scan: 16 sync-invariant finding\(s\) vs origin/next$'
[[ "$(tail -1 <<<"$OUT")" =~ ^scan\ covered\ [1-9][0-9]*\ file ]] && ok "the last line is still the coverage line" || bad "last line: $(tail -1 <<<"$OUT")"
run_scan next
finding "a base named on the command line is used" '^review-scan: 16 sync-invariant finding\(s\) vs next$'
run_scan no-such-ref
if [[ $RC -eq 1 ]] && grep -q 'SCAN FAILED' <<<"$OUT"; then ok "an unknown base ref fails the scan"; else bad "unknown base (want 1 and SCAN FAILED, got $RC: $OUT)"; fi

echo "Layer 2, patterns over whole changed files:"
g checkout -q -b feat/1/scan next
# put PATH CONTENT: write a file into the branch
put() { mkdir -p "$REPO/$(dirname "$1")" && printf '%b' "$2" > "$REPO/$1"; }
put backend/src/raw.rs 'fn f() {\n    dbg!(x);\n    let _ = send();\n    rx.recv().ok();\n    sqlx::query(&format!("select * from {t}"));\n    #[allow(dead_code)]\n    todo!()\n}\n#[test]\n#[ignore]\nfn a() { assert!(!v.contains(&1)); }\n#[ignore = "todo: write it"]\nfn b() { assert!(m.get(&k).is_none()); }\n#[ignore = "network: downloads NASR"]\nfn c() { assert!(v.contains(&1)); }\nfn d() { let x = format!("{} rows", n); }\n'
put web/src/page.tsx 'console.log(x);\n// @ts-expect-error -- legacy\n<div dangerouslySetInnerHTML={{ __html: h }} />\nconsole.error(x);\n'
put web/src/lib/x.ts '  debugger;\n// eslint-disable-next-line\nconst debuggerPanel = 1;\n'
put web/src/lib/x.test.ts 'it.only("a", () => {});\nexpect(a).not.toContain("x");\nexpect(f).toHaveBeenCalledTimes(0);\nexpect(a).toContain("y");\ntest.skip("b", () => {});\n'
put packages/api-client/src/generated/schema.d.ts 'console.log(x);\n'
put .claude/rules/code-quality.md 'Never leave dbg!( or console.log( in a diff.\n'
put CLAUDE.md 'No dbg!( calls.\n'
put bin/lib/scan.test.sh "put backend/src/a.rs 'dbg!(x);'\n"
put bin/review-scan.sh 'check "debug output" -e "dbg!(" -e "console.log("\n'
g add -A && g commit -qm files
put web/src/unstaged.ts 'console.log("untracked");\n'
run_scan
[[ $RC -eq 0 ]] && ok "the scan exits 0 with hits" || bad "scan exit (want 0, got $RC): $OUT"

# flagged NAME FILE:LINE MESSAGE-FRAGMENT: the hit is among the lines a check prints before its `^ message`
flagged() {
    if awk -v loc="$2:" -v msg="^ $3" '
        index($0, "^ ") == 1 { if (index($0, msg) == 1 && seen) hit = 1; seen = 0; next }
        index($0, loc) == 1 { seen = 1 }
        END { exit !hit }' <<<"$OUT"; then ok "$1"; else bad "$1 (no '$3' after $2 in: $OUT)"; fi
}
# unflagged NAME FILE:LINE: no hit at all on that line
unflagged() { if grep -q "^$2:" <<<"$OUT"; then bad "$1 ($2 in: $OUT)"; else ok "$1"; fi; }
flagged "dbg!"                                "backend/src/raw.rs:2"  "debug output must be removed"
flagged "let _ ="                             "backend/src/raw.rs:3"  "swallowed error"
flagged ".ok();"                              "backend/src/raw.rs:4"  "swallowed error"
flagged "SQL built with format!"              "backend/src/raw.rs:5"  "SQL built with format!"
flagged "#[allow]"                            "backend/src/raw.rs:6"  "#[allow(...)] needs a stated reason"
flagged "todo!()"                             "backend/src/raw.rs:7"  "test skeleton left in"
flagged "a bare #[ignore]"                    "backend/src/raw.rs:10" "test skeleton left in"
flagged "assert!(!...)"                       "backend/src/raw.rs:11" "negative assertion"
flagged "an #[ignore] whose reason is todo"   "backend/src/raw.rs:12" "test skeleton left in"
flagged "assert!(....is_none())"              "backend/src/raw.rs:13" "negative assertion"
unflagged "an #[ignore] with a reason, over a positive assertion" "backend/src/raw.rs:14"
unflagged "a positive assertion"              "backend/src/raw.rs:15"
unflagged "format! that builds no SQL"        "backend/src/raw.rs:16"
flagged "console.log"                         "web/src/page.tsx:1"    "debug output must be removed"
flagged "@ts-expect-error"                    "web/src/page.tsx:2"    "@ts-ignore, @ts-expect-error and eslint-disable"
flagged "dangerouslySetInnerHTML"             "web/src/page.tsx:3"    "dangerouslySetInnerHTML"
unflagged "console.error"                     "web/src/page.tsx:4"
flagged "a debugger statement"                "web/src/lib/x.ts:1"    "debug output must be removed"
flagged "eslint-disable"                      "web/src/lib/x.ts:2"    "@ts-ignore, @ts-expect-error and eslint-disable"
unflagged "an identifier containing debugger" "web/src/lib/x.ts:3"
flagged "it.only"                             "web/src/lib/x.test.ts:1" "focused, skipped or todo test"
flagged ".not.toContain"                      "web/src/lib/x.test.ts:2" "negative assertion"
flagged "toHaveBeenCalledTimes(0)"            "web/src/lib/x.test.ts:3" "negative assertion"
unflagged "a positive test assertion"         "web/src/lib/x.test.ts:4"
flagged "test.skip"                           "web/src/lib/x.test.ts:5" "focused, skipped or todo test"
flagged "an untracked file is scanned"        "web/src/unstaged.ts:1" "debug output must be removed"
unflagged "a generated .d.ts is not read"     "packages/api-client/src/generated/schema.d.ts"
unflagged "guidance naming the patterns is not read" ".claude/rules/code-quality.md"
unflagged "the root CLAUDE.md is not read"    "CLAUDE.md"
unflagged "a shell test's fixture is not read" "bin/lib/scan.test.sh"
unflagged "the scan script's own pattern list is not read" "bin/review-scan.sh"
if grep -q 'CRITICAL: .env is in the diff' <<<"$OUT"; then bad "no .env, no CRITICAL ($OUT)"; else ok "no .env, no CRITICAL"; fi
[[ "$(tail -1 <<<"$OUT")" == "scan covered 10 file(s)" ]] && ok "the last line counts committed and untracked files" || bad "coverage line (want 10, got: $(tail -1 <<<"$OUT"))"

printf 'DATABASE_URL=postgres://x\n' > "$REPO/.env"
g add -f .env
run_scan
if grep -q 'CRITICAL: .env is in the diff' <<<"$OUT"; then ok "a force-added .env is called out"; else bad "a force-added .env is called out ($OUT)"; fi
g rm -q --cached .env && rm "$REPO/.env"

if [[ "$(id -u)" != 0 ]]; then
    chmod 000 "$REPO/backend/src/raw.rs"
    run_scan
    chmod 644 "$REPO/backend/src/raw.rs"
    if [[ $RC -eq 1 ]] && grep -q 'SCAN FAILED' <<<"$OUT"; then ok "an unreadable file fails the scan"; else bad "unreadable file (want 1 and SCAN FAILED, got $RC: $OUT)"; fi
fi

echo ""
echo "review-scan: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
