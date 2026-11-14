//! Docs that cannot drift from the code (VATUSA/OIS#592).
//!
//! - **The permission map** (`docs-site/reference/api-permissions.md`) is generated from the handlers'
//!   `RequirePermission<P>` markers and pinned here, so it is never hand-maintained. Regenerate with
//!   `just docs-permissions` (or `OIS_REGENERATE_DOCS=1` on this test).
//! - **Every API example** in a docs-site code block is checked against the real contract, and every
//!   `GET` example is sent through the real router. A renamed endpoint, or an example written from
//!   memory, fails CI.
//!
//! Both read source files rather than trusting a hand-kept list, and both cross-check what they read
//! against something independent (the permission catalog, the OpenAPI document), so a scanner that
//! quietly misread the source would fail rather than publish a wrong answer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use utoipa::OpenApi;

fn backend_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    backend_dir().parent().unwrap().to_path_buf()
}

/// The index just past the `)` that closes the `(` at `open`, skipping string literals — the
/// attribute descriptions contain parentheses of their own.
fn close_paren(src: &str, open: usize) -> usize {
    let bytes = src.as_bytes();
    let (mut depth, mut i, mut in_str) = (0usize, open, false);
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            match c {
                b'\\' => i += 1,
                b'"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                b'"' => in_str = true,
                // A char literal — `')'`, `'\\''` — must be skipped whole so its paren is not
                // counted (`permissions.rs` has `.find(')')` since #586). A lifetime (`&'static str`)
                // opens with the same byte and must NOT be skipped, so only advance past something
                // that actually closes like a char literal.
                b'\'' => {
                    let esc = bytes.get(i + 1) == Some(&b'\\');
                    let end = if esc { i + 3 } else { i + 2 };
                    if bytes.get(end) == Some(&b'\'') {
                        i = end;
                    }
                }
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    panic!("unbalanced parentheses from byte {open}");
}

/// `Type → "segment.….action"` for every `permission!(Type, ["segment", …], Action)` declaration.
fn permission_names(src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (at, _) in src.match_indices("permission!(") {
        let open = at + "permission!".len();
        let body = &src[open + 1..close_paren(src, open) - 1];
        let (name, rest) = body.split_once(',').expect("permission!(Name, …)");
        let segments: Vec<&str> = rest[rest.find('[').unwrap() + 1..rest.find(']').unwrap()]
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
            .collect();
        let action = rest[rest.find(']').unwrap() + 1..]
            .trim_matches(|c: char| c == ',' || c.is_whitespace())
            .to_ascii_lowercase();
        out.insert(
            name.trim().to_string(),
            format!("{}.{action}", segments.join(".")),
        );
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Route {
    path: String,
    method: String,
    /// Permission names the handler requires through `RequirePermission<P>`, in signature order.
    permissions: Vec<String>,
}

/// Every `#[utoipa::path(...)]`-documented handler in `handlers/*.rs`, with the permission markers in
/// the signature that follows it.
fn scan_routes(names: &BTreeMap<String, String>) -> Vec<Route> {
    let mut routes = Vec::new();
    let dir = backend_dir().join("src/handlers");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .filter(|p| {
            !p.file_stem()
                .is_some_and(|s| s.to_string_lossy().ends_with("_tests"))
        })
        .collect();
    files.sort();
    for file in files {
        let src = std::fs::read_to_string(&file).unwrap();
        for (at, _) in src.match_indices("#[utoipa::path(") {
            let open = at + "#[utoipa::path".len();
            let end = close_paren(&src, open);
            let attr = &src[open + 1..end - 1];
            let method = attr
                .trim_start()
                .split(|c: char| c == ',' || c.is_whitespace())
                .next()
                .unwrap()
                .to_ascii_uppercase();
            let path_at =
                attr.find("path = \"").expect("utoipa path attribute") + "path = \"".len();
            let path = attr[path_at..path_at + attr[path_at..].find('"').unwrap()].to_string();

            let fn_at = end
                + src[end..]
                    .find("fn ")
                    .expect("a handler follows its attribute");
            let params_open = fn_at + src[fn_at..].find('(').unwrap();
            let params = &src[params_open..close_paren(&src, params_open)];
            let mut permissions = Vec::new();
            for (p, _) in params.match_indices("RequirePermission<") {
                let ty = &params[p + "RequirePermission<".len()..];
                let ty = &ty[..ty.find('>').unwrap()];
                let ty = ty.rsplit("::").next().unwrap().trim();
                permissions.push(
                    names
                        .get(ty)
                        .unwrap_or_else(|| {
                            panic!("{}: unknown permission marker {ty}", file.display())
                        })
                        .clone(),
                );
            }
            routes.push(Route {
                path,
                method,
                permissions,
            });
        }
    }
    routes.sort();
    routes
}

/// `(METHOD, path)` for every operation the published OpenAPI document describes.
fn openapi_operations() -> BTreeSet<(String, String)> {
    let doc = crate::openapi::ApiDoc::openapi();
    let mut ops = BTreeSet::new();
    for (path, item) in &doc.paths.paths {
        for (method, op) in [
            ("GET", &item.get),
            ("POST", &item.post),
            ("PUT", &item.put),
            ("PATCH", &item.patch),
            ("DELETE", &item.delete),
        ] {
            if op.is_some() {
                ops.insert((method.to_string(), path.clone()));
            }
        }
    }
    ops
}

const BEGIN: &str = "<!-- generated:permission-map:begin -->";
const END: &str = "<!-- generated:permission-map:end -->";

fn render_map(routes: &[Route]) -> String {
    let mut out = String::from("| Endpoint | Method | Requires |\n| --- | --- | --- |\n");
    for r in routes {
        let requires = if r.permissions.is_empty() {
            "— (no permission marker)".to_string()
        } else {
            r.permissions
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(" + ")
        };
        out.push_str(&format!("| `{}` | {} | {requires} |\n", r.path, r.method));
    }
    out
}

fn permissions_source() -> String {
    std::fs::read_to_string(backend_dir().join("src/auth/permissions.rs")).unwrap()
}

/// The scanner derives each name the same way the macro does; if it ever stopped doing so, a name
/// would fall outside the catalog and this would fail rather than publish it.
#[test]
fn every_derived_permission_name_is_in_the_catalog() {
    let names = permission_names(&permissions_source());
    assert!(
        names.len() >= 50,
        "scanned only {} permission markers",
        names.len()
    );
    let catalog: BTreeSet<&str> = ois_core::catalog::draft_new_permission_names()
        .into_iter()
        .collect();
    let unknown: Vec<_> = names
        .iter()
        .filter(|(_, n)| !catalog.contains(n.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "derived names missing from the catalog: {unknown:?}"
    );
}

/// The source scan and the served OpenAPI document must describe the same set of operations — so the
/// map can neither miss an endpoint nor invent one.
#[test]
fn the_scanned_routes_are_exactly_the_documented_operations() {
    let scanned: BTreeSet<(String, String)> = scan_routes(&permission_names(&permissions_source()))
        .into_iter()
        .map(|r| (r.method, r.path))
        .collect();
    let documented = openapi_operations();
    let missed: Vec<_> = documented.difference(&scanned).collect();
    let invented: Vec<_> = scanned.difference(&documented).collect();
    assert!(missed.is_empty(), "documented but not scanned: {missed:?}");
    assert!(
        invented.is_empty(),
        "scanned but not documented: {invented:?}"
    );
}

#[test]
fn the_published_permission_map_is_current() {
    let file = repo_root().join("docs-site/reference/api-permissions.md");
    let page = std::fs::read_to_string(&file).unwrap();
    let start = page.find(BEGIN).expect("begin marker") + BEGIN.len();
    let end = page.find(END).expect("end marker");
    let table = render_map(&scan_routes(&permission_names(&permissions_source())));
    let expected = format!("\n{table}");
    if page[start..end] == expected {
        return;
    }
    if std::env::var_os("OIS_REGENERATE_DOCS").is_some() {
        std::fs::write(
            &file,
            format!("{}{expected}{}", &page[..start], &page[end..]),
        )
        .unwrap();
        return;
    }
    panic!(
        "docs-site/reference/api-permissions.md is out of date with the handlers' permission \
         markers. Regenerate it with `just docs-permissions` and commit the result."
    );
}

// ---- docs examples ---------------------------------------------------------------------------------

/// One `/api/v1/…` request found in a fenced code block.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Example {
    file: PathBuf,
    method: String,
    path: String,
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && name != "node_modules" {
                markdown_files(&path, out);
            }
        } else if name.ends_with(".md") {
            out.push(path);
        }
    }
}

/// Every API request in a fenced code block: the method from `curl -X` (GET otherwise), and the path
/// up to its query string. Prose and inline code are not examples; `/docs/api/v1/…` is the spec, not
/// the API, and a path in [`NOT_OPENAPI`] is a real route that is deliberately no OpenAPI operation.
fn examples_in(file: &Path) -> Vec<Example> {
    let src = std::fs::read_to_string(file).unwrap();
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut block = String::new();
    for line in src.lines() {
        if line.trim_start().starts_with("```") {
            if in_fence {
                // One command per line, with `\`-continued lines joined — the method belongs to the
                // command it is on, not to the whole block.
                for command in block.replace("\\\n", " ").lines() {
                    let method = command
                        .split_whitespace()
                        .skip_while(|w| *w != "-X")
                        .nth(1)
                        .unwrap_or("GET")
                        .to_ascii_uppercase();
                    for (at, _) in command.match_indices("/api/v1/") {
                        if command[..at].ends_with("/docs") {
                            continue;
                        }
                        let path: String = command[at..]
                            .chars()
                            .take_while(|c| {
                                !c.is_whitespace() && !matches!(c, '"' | '\'' | '?' | '\\')
                            })
                            .collect();
                        if NOT_OPENAPI.iter().any(|(route, _)| *route == path) {
                            continue;
                        }
                        out.push(Example {
                            file: file.to_path_buf(),
                            method: method.clone(),
                            path,
                        });
                    }
                }
                block.clear();
            }
            in_fence = !in_fence;
        } else if in_fence {
            block.push_str(line);
            block.push('\n');
        }
    }
    out
}

/// Real routes the docs may show that are deliberately not OpenAPI operations, each with why. Anything
/// else under `/api/v1/` in a docs example must be a documented operation — including a websocket URL,
/// so a mistyped socket path is still caught (#686).
const NOT_OPENAPI: &[(&str, &str)] = &[(
    "/api/v1/ws",
    "the realtime socket: a websocket upgrade, deliberately not an OpenAPI operation (#589)",
)];

fn docs_examples(root: &Path) -> Vec<Example> {
    let mut files = Vec::new();
    markdown_files(root, &mut files);
    files.sort();
    files.iter().flat_map(|f| examples_in(f)).collect()
}

/// Whether a concrete path matches an OpenAPI template (`{param}` matches one segment).
fn matches_template(path: &str, template: &str) -> bool {
    let (p, t): (Vec<&str>, Vec<&str>) = (path.split('/').collect(), template.split('/').collect());
    p.len() == t.len()
        && p.iter()
            .zip(&t)
            .all(|(a, b)| (b.starts_with('{') && b.ends_with('}') && !a.is_empty()) || a == b)
}

fn undocumented(examples: &[Example]) -> Vec<String> {
    let ops = openapi_operations();
    examples
        .iter()
        .filter(|e| {
            !ops.iter()
                .any(|(m, t)| *m == e.method && matches_template(&e.path, t))
        })
        .map(|e| format!("{} {} ({})", e.method, e.path, e.file.display()))
        .collect()
}

/// Every example names an operation that exists, with the method it uses.
#[test]
fn every_docs_example_is_a_documented_operation() {
    let examples = docs_examples(&repo_root().join("docs-site"));
    assert!(
        !examples.is_empty(),
        "no API examples found — has the scanner stopped reading the docs?"
    );
    let bad = undocumented(&examples);
    assert!(
        bad.is_empty(),
        "docs examples that are not real operations: {bad:?}"
    );
}

/// The realtime socket is documented with `wss://…/api/v1/ws`, which is no OpenAPI operation; it is
/// not an example to check — but an HTTP call in the same block still is.
#[test]
fn a_websocket_url_is_not_an_api_example() {
    let dir = std::env::temp_dir().join(format!("ois-docs-ws-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("page.md"),
        "```js\nnew WebSocket(\"wss://<your-ois-host>/api/v1/ws\", [\"ois.v1\"]);\nconst s = new WebSocket('ws://localhost:8080/api/v1/ws');\n```\n\n```bash\ncurl https://ois/api/v1/flow/no-such-thing\n```\n",
    )
    .unwrap();
    let examples = docs_examples(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    let paths: Vec<_> = examples.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["/api/v1/flow/no-such-thing"]);
}

/// The exemption is the route, not the scheme: a websocket URL to any other path is an example like any
/// other, so a mistyped socket path in the docs fails the guard instead of slipping through (#686).
#[test]
fn only_the_named_socket_route_is_exempt() {
    let dir = std::env::temp_dir().join(format!(
        "ois-docs-ws-typo-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("page.md"),
        "```js\nnew WebSocket(\"wss://<your-ois-host>/api/v1/wss\", [\"ois.v1\"]);\n```\n",
    )
    .unwrap();
    let examples = docs_examples(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    let paths: Vec<_> = examples.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["/api/v1/wss"]);
}

/// The check itself works: a page with a made-up endpoint is caught. A throwaway directory, so no real
/// page is touched.
#[test]
fn a_made_up_endpoint_in_the_docs_is_caught() {
    let dir = std::env::temp_dir().join(format!("ois-docs-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("probe.md"),
        "Inline `/api/v1/not/an/example` is prose.\n\n```bash\ncurl -X POST https://ois/api/v1/flow/no-such-thing\ncurl https://ois/api/v1/flow/traffic\n```\n",
    )
    .unwrap();
    let examples = docs_examples(&dir);
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(
        examples.len(),
        2,
        "only fenced examples count: {examples:?}"
    );
    assert_eq!(
        undocumented(&examples),
        [format!(
            "POST /api/v1/flow/no-such-thing ({})",
            dir.join("probe.md").display()
        )]
    );
}

/// Every runnable `GET` example is sent through the real router, authenticated by an API key that holds
/// every permission a key may hold, nationally. It must reach a handler: not 404 or 405 (a path that
/// isn't routed) and not 401 (a permission the example's endpoint needs that a key can't hold). A 503
/// is a handler answering that the test has no live feed, which is fine.
///
/// An example with a placeholder (`{id}`, `<…>`) names a resource the reader supplies; it cannot be
/// sent as written, so it is held to the contract check above instead.
#[sqlx::test]
async fn every_get_example_reaches_a_handler(pool: sqlx::PgPool) {
    use tower::ServiceExt;

    let owner = crate::scope_test_support::seed_user(&pool).await;
    sqlx::query(
        "insert into access.user_roles (user_id, role_name, source) values ($1, 'SERVER_ADMIN', 'system')",
    )
        .bind(&owner)
        .execute(&pool)
        .await
        .unwrap();
    let token = "ois_pat_docs-examples";
    let key: String = sqlx::query_scalar(
        "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
         values ($1, 'docs', 'ois_pat_docs', $2) returning id",
    )
    .bind(&owner)
    .bind(crate::repos::access::sha256_hex(token))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into access.api_key_permissions (api_key_id, permission_name) \
         select $1, name from access.permissions where name not like 'api\\_keys.%'",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .unwrap();
    let state = crate::scope_test_support::test_state(pool, Default::default());

    let gets: Vec<Example> = docs_examples(&repo_root().join("docs-site"))
        .into_iter()
        .filter(|e| e.method == "GET" && !e.path.contains(['{', '<']))
        .collect();
    assert!(!gets.is_empty());
    for e in gets {
        let request = axum::http::Request::builder()
            .uri(&e.path)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let status = crate::router::build_router(state.clone())
            .oneshot(request)
            .await
            .unwrap()
            .status()
            .as_u16();
        assert!(
            !matches!(status, 401 | 404 | 405),
            "{} {} ({}) answered {status}",
            e.method,
            e.path,
            e.file.display()
        );
    }
}

// ---- #814 AC1, AC7: the prediction design doc cites its constants and its sources -------------

const PREDICTION_DOC: &str = "docs/architecture/prediction.md";

fn prediction_doc() -> String {
    let path = repo_root().join(PREDICTION_DOC);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn is_constant_name(s: &str) -> bool {
    s.len() > 2
        && s.starts_with(|c: char| c.is_ascii_uppercase())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Every constant the doc cites as `` `NAME` (`path.rs:line`) ``, whitespace (a line break included)
/// allowed between the two: `(name, path, line)`.
fn constant_citations(doc: &str) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    let parts: Vec<&str> = doc.split('`').collect();
    // Odd indices are inside backticks: `NAME`, then " (" or "\n  (", then `path:line`, then ")".
    let mut i = 1;
    while i + 2 < parts.len() {
        let (name, between, cite) = (parts[i], parts[i + 1], parts[i + 2]);
        let after = parts.get(i + 3).copied().unwrap_or("");
        if is_constant_name(name)
            && between.trim() == "("
            && after.starts_with(')')
            && let Some((path, line)) = cite.rsplit_once(':')
            && path.ends_with(".rs")
            && let Ok(line) = line.parse::<usize>()
        {
            out.push((name.to_string(), path.to_string(), line));
        }
        i += 2;
    }
    out
}

/// Whether `line` declares `name` as a `const` or `static`.
fn declares(line: &str, name: &str) -> bool {
    ["const ", "static "].iter().any(|kw| {
        line.split(kw)
            .nth(1)
            .is_some_and(|rest| rest.strip_prefix(name).is_some_and(|r| r.starts_with(':')))
    })
}

/// Each citation whose line does not declare its constant, as `NAME path:line: why`.
fn miscited(root: &Path, citations: &[(String, String, usize)]) -> Vec<String> {
    citations
        .iter()
        .filter_map(|(name, path, line)| {
            let src = match std::fs::read_to_string(root.join(path)) {
                Ok(s) => s,
                Err(e) => return Some(format!("{name} {path}:{line}: {e}")),
            };
            match src.lines().nth(line - 1) {
                Some(l) if declares(l, name) => None,
                Some(l) => Some(format!("{name} {path}:{line}: `{}`", l.trim())),
                None => Some(format!("{name} {path}:{line}: past the end")),
            }
        })
        .collect()
}

/// Every `const`/`static` name declared in Rust source under `dir`.
fn declared_constants(dir: &Path, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            declared_constants(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let src = std::fs::read_to_string(&path).unwrap();
            for line in src.lines() {
                for kw in ["const ", "static "] {
                    if let Some(rest) = line.split(kw).nth(1) {
                        let name: String = rest
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if is_constant_name(&name) && rest[name.len()..].starts_with(':') {
                            out.insert(name);
                        }
                    }
                }
            }
        }
    }
}

/// The backticked constant names in `doc` that are declared in the code but carry no citation.
fn uncited(doc: &str, declared: &BTreeSet<String>) -> Vec<String> {
    let cited: BTreeSet<String> = constant_citations(doc).into_iter().map(|c| c.0).collect();
    let named: BTreeSet<&str> = doc.split('`').skip(1).step_by(2).collect();
    named
        .into_iter()
        .filter(|n| declared.contains(*n) && !cited.contains(*n))
        .map(str::to_string)
        .collect()
}

/// The line `name` is declared on in `path`, read from the source rather than typed.
fn declaring_line(path: &str, name: &str) -> usize {
    let src = std::fs::read_to_string(repo_root().join(path)).unwrap();
    src.lines().position(|l| declares(l, name)).unwrap() + 1
}

#[test]
fn the_prediction_doc_cites_each_constant_where_it_is_defined() {
    let doc = prediction_doc();
    let citations = constant_citations(&doc);
    assert!(citations.len() >= 15, "the scanner is live: {citations:?}");
    let names: BTreeSet<&str> = citations.iter().map(|c| c.0.as_str()).collect();
    for name in [
        "PROPOSED_WHEELS_UP_MIN",
        "AIRBORNE_GS_KT",
        "GROUND_ROUTE_FACTOR",
        "ANCHOR_MAX_BIAS",
        "ANCHOR_MIN_GS_KT",
        "ANCHOR_ALT_TOLERANCE_FT",
        "RELEASE_MARGIN_MS",
    ] {
        assert!(names.contains(name), "{name} is cited");
    }
    let wrong = miscited(&repo_root(), &citations);
    assert!(
        wrong.is_empty(),
        "cited to a line that does not define it:\n{wrong:#?}"
    );
}

#[test]
fn a_constant_cited_to_the_wrong_line_is_caught() {
    let path = "backend/src/feed/predict.rs";
    let right = declaring_line(path, "PROPOSED_WHEELS_UP_MIN");
    let airborne = declaring_line(path, "AIRBORNE_GS_KT");
    let page = format!(
        "`PROPOSED_WHEELS_UP_MIN` (`{path}:{right}`) and\n`AIRBORNE_GS_KT`\n  (`{path}:{}`).",
        airborne + 1
    );
    let citations = constant_citations(&page);
    assert_eq!(
        citations.len(),
        2,
        "both found, the wrapped one too: {citations:?}"
    );
    let wrong = miscited(&repo_root(), &citations);
    assert_eq!(wrong.len(), 1, "{wrong:?}");
    assert!(wrong[0].starts_with("AIRBORNE_GS_KT "), "{wrong:?}");
}

#[test]
fn every_constant_the_prediction_doc_names_is_cited() {
    let mut declared = BTreeSet::new();
    declared_constants(&backend_dir().join("src"), &mut declared);
    declared_constants(&repo_root().join("crates"), &mut declared);
    assert!(declared.len() > 100, "the scan is live: {}", declared.len());
    assert!(declared.contains("PROPOSED_WHEELS_UP_MIN"));
    let missing = uncited(&prediction_doc(), &declared);
    assert!(missing.is_empty(), "named without a citation: {missing:?}");
    // The check, on a planted page: a declared constant with no citation is caught, an abbreviation
    // that declares nothing is not.
    let planted = "`PROPOSED_WHEELS_UP_MIN` is ten minutes. `RDY` means ready.";
    assert_eq!(uncited(planted, &declared), vec!["PROPOSED_WHEELS_UP_MIN"]);
}

/// The section of `doc` under `## heading`, to the next `## `.
fn section<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("no `## {heading}` section"));
    let body = &doc[start + heading.len() + 5..];
    body.find("\n## ").map_or(body, |end| &body[..end])
}

/// The cells of the comparison table's row whose first cell is `topic`.
fn comparison_row<'a>(doc: &'a str, topic: &str) -> Vec<&'a str> {
    let line = section(doc, "How TBFM and vTBFM differ")
        .lines()
        .find(|l| l.starts_with(&format!("| {topic} |")))
        .unwrap_or_else(|| panic!("no `{topic}` row"));
    line.trim_matches('|').split('|').map(str::trim).collect()
}

#[test]
fn the_prediction_doc_covers_each_topic_and_cites_its_sources() {
    let doc = prediction_doc();
    for heading in [
        "What the model reads",
        "Wheels-up",
        "Climb, cruise and descent",
        "Winds",
        "Groundspeed anchoring",
        "Freeze horizons",
        "The release time",
        "How TBFM and vTBFM differ",
        "Sources",
    ] {
        assert_eq!(
            doc.matches(&format!("\n## {heading}\n")).count(),
            1,
            "{heading}"
        );
    }
    let freeze = section(&doc, "Freeze horizons");
    assert!(freeze.contains("There is no freeze horizon"), "{freeze}");
    assert!(freeze.contains("an FCA release"), "{freeze}");

    // Columns: topic, OIS, TBFM, vTBFM. Each TBFM claim sits beside its source.
    let freeze_row = comparison_row(&doc, "Freeze");
    assert!(freeze_row[2].contains("STA freezes") && freeze_row[2].contains("ETA keeps updating"));
    assert!(freeze_row[2].contains("ATC-364 §2,"), "{freeze_row:?}");
    let perf = comparison_row(&doc, "Performance");
    assert!(perf[2].contains("trajectory synthesizer") && perf[2].contains("Swenson §2.1"));
    let departure = comparison_row(&doc, "Departure");
    assert!(
        departure[2].contains("depart the runway") && departure[2].contains("NTRS 20180006743 §II")
    );

    // AC7: OIS's anchoring and vTBFM's bias, each in its own column.
    let anchoring = comparison_row(&doc, "Anchoring");
    for part in ["±15 %", "≥250 kt", "300 ft", "whole remaining flight"] {
        assert!(anchoring[1].contains(part), "OIS {part}: {anchoring:?}");
    }
    for part in ["0.85 to 1.15", "≥250 kt", "500 ft", "whole integration"] {
        assert!(anchoring[3].contains(part), "vTBFM {part}: {anchoring:?}");
    }

    // vTBFM by manual section and commit, the same commit `PROPOSED_WHEELS_UP_MIN` cites.
    let predict = std::fs::read_to_string(backend_dir().join("src/feed/predict.rs")).unwrap();
    let sha = predict
        .split("`zla-artcc/vTBFM` `")
        .nth(1)
        .and_then(|r| r.split('`').next())
        .expect("predict.rs cites a vTBFM commit");
    let sources = section(&doc, "Sources");
    assert!(
        sources.contains(&format!("`zla-artcc/vTBFM` at `{sha}`")),
        "the doc cites vTBFM at {sha}: {sources}"
    );
    assert!(
        sources.contains("§17.2") && sources.contains("§18.2"),
        "{sources}"
    );
}
