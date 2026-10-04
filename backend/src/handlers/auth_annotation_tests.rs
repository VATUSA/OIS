//! VATUSA/OIS#586: a handler's OpenAPI annotation must tell the truth about its gate, and every
//! handler that answers without a credential must be public on purpose.
//!
//! Nine flow reads advertised `(status = 401)` while taking no credential at all, so the spec told an
//! integrator they needed auth while the code gave them away. Nothing checked the two against each
//! other, so this scans every `#[utoipa::path]` handler in `src/handlers/` and asserts:
//!
//! - **(a)** advertising 401 ⇒ the handler takes an auth extractor (or is a listed exception);
//! - **(b)** taking `RequirePermission` ⇒ it advertises 401;
//! - **(c)** taking no auth extractor ⇒ it is in [`PUBLIC`] with a reason — and nothing listed there
//!   has quietly gained one, so the list can't rot.

use std::collections::BTreeSet;

/// Every handler reachable with no credential, and why that is deliberate. Adding an unauthenticated
/// handler means adding it here — a decision, not an accident.
#[rustfmt::skip]
const PUBLIC: &[(&str, &str, &str)] = &[
    ("airports", "get_airport", "an airport's published position under /public/, from the public airport dataset"),
    ("atc", "list_atc", "the ATC overlay on the public FCA overview and facility map"),
    ("atc", "list_flow_facilities", "the facility directory behind public search and the facility map"),
    ("auth", "vatsim_login", "starts sign-in, so there is no credential yet"),
    ("auth", "vatsim_callback", "completes sign-in, so there is no credential yet"),
    ("auth", "desktop_exchange", "trades a one-time desktop code for a session; the code is the credential"),
    ("desktop", "download", "the desktop installer redirect on the public download page"),
    ("facilities", "list_facilities", "public facility metadata (the facility map)"),
    ("facilities", "get_facility", "public facility metadata (the facility map)"),
    ("flow", "list_fcas", "the public FCA overview (/advisories/fcas)"),
    ("flow", "fca_traffic", "the public FCA overview (/advisories/fcas)"),
    ("flow", "fca_counts", "the public FCA overview (/advisories/fcas)"),
    ("flow", "list_routes", "the public FCA overview and facility map"),
    ("flow", "aircraft_route", "the public FCA overview and facility map"),
    ("flow", "route_coverage", "the public FCA overview's coverage panel"),
    ("flow", "data_status", "the public FCA overview's data-freshness indicator"),
    ("flow", "list_traffic", "live traffic on the public FCA overview, facility map and /pilot"),
    ("flow", "projected_traffic", "the public FCA overview's time slider"),
    ("flow", "flight_advisory", "the pilot-facing /pilot lookup"),
    ("health", "health", "liveness probe"),
    ("public", "get_board", "the public advisories board"),
];

/// Handlers that advertise 401 without an auth extractor, because they reject a bad credential they
/// receive in the body rather than through the request's identity.
const ADVERTISE_401_WITHOUT_EXTRACTOR: &[(&str, &str)] = &[("auth", "desktop_exchange")];

/// What resolves or requires a caller's identity. `: Actor` is #583's extractor, counted ahead of its
/// merge so the scan doesn't misread its handlers as public.
const AUTH_EXTRACTORS: &[&str] = &[
    "RequirePermission<",
    "CurrentUser",
    "CurrentApiKey",
    "CurrentServiceAccount",
    "SessionToken",
    ":Actor",
];

struct Handler {
    file: String,
    name: String,
    advertises_401: bool,
    takes_permission: bool,
    takes_auth: bool,
}

/// From `open` (just past an opening paren) to just past its matching close.
fn balanced(src: &str, open: usize) -> usize {
    let mut depth = 1;
    for (i, c) in src[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return open + i + 1;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parens after byte {open}");
}

fn handlers_in(file: &str, src: &str) -> Vec<Handler> {
    const ANNOTATION: &str = "#[utoipa::path(";
    const FN: &str = "pub async fn ";
    let squash = |s: &str| s.split_whitespace().collect::<String>();

    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = src[at..].find(ANNOTATION).map(|i| at + i) {
        let annotation_end = balanced(src, start + ANNOTATION.len());
        let annotation = squash(&src[start..annotation_end]);
        let name_start = src[annotation_end..]
            .find(FN)
            .map(|i| annotation_end + i + FN.len())
            .expect("an annotation is followed by its handler");
        // A handler that isn't `pub async fn` would otherwise be judged by the next one's signature.
        let next_annotation = src[annotation_end..]
            .find(ANNOTATION)
            .map(|i| annotation_end + i);
        assert!(
            next_annotation.is_none_or(|next| name_start < next),
            "{file}.rs: the annotation at byte {start} isn't on a `pub async fn`; the scan can't pair it"
        );
        let params_start = src[name_start..].find('(').map(|i| name_start + i).unwrap();
        let name = src[name_start..params_start].split('<').next().unwrap();
        let params = squash(&src[params_start..balanced(src, params_start + 1)]);

        out.push(Handler {
            file: file.to_string(),
            name: name.trim().to_string(),
            advertises_401: annotation.contains("status=401"),
            takes_permission: params.contains("RequirePermission<"),
            takes_auth: AUTH_EXTRACTORS.iter().any(|e| params.contains(e)),
        });
        at = annotation_end;
    }
    out
}

fn scan() -> Vec<Handler> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/handlers");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        // Only top-level files are read; a handler moved into a subdirectory would go unchecked.
        assert!(
            !path.is_dir(),
            "{path:?}: the scan doesn't descend into handler subdirectories"
        );
        let file = path.file_stem().unwrap().to_string_lossy().into_owned();
        // Test modules hold source-scan literals like this one's, not handlers.
        if path.extension().is_some_and(|e| e == "rs") && !file.ends_with("_tests") {
            out.extend(handlers_in(&file, &std::fs::read_to_string(&path).unwrap()));
        }
    }

    // Without these the test passes by checking nothing the moment the matcher stops matching.
    assert!(
        out.len() >= 150,
        "only {} annotated handlers found; the scan is broken",
        out.len()
    );
    assert!(
        out.iter()
            .any(|h| h.file == "flow" && h.name == "list_traffic"),
        "the scan no longer finds a handler known to exist"
    );
    out
}

fn ids<'a>(handlers: impl Iterator<Item = &'a Handler>) -> BTreeSet<String> {
    handlers
        .map(|h| format!("{}::{}", h.file, h.name))
        .collect()
}

#[test]
fn no_handler_advertises_401_without_a_gate() {
    let allowed: BTreeSet<String> = ADVERTISE_401_WITHOUT_EXTRACTOR
        .iter()
        .map(|(f, n)| format!("{f}::{n}"))
        .collect();
    let lying = &ids(scan().iter().filter(|h| h.advertises_401 && !h.takes_auth)) - &allowed;
    assert!(
        lying.is_empty(),
        "these advertise 401 but answer without a credential — drop the 401, or gate them: {lying:?}"
    );
}

#[test]
fn every_gated_handler_advertises_401() {
    let silent = ids(scan()
        .iter()
        .filter(|h| h.takes_permission && !h.advertises_401));
    assert!(
        silent.is_empty(),
        "these require a permission but don't say a caller can get 401: {silent:?}"
    );
}

#[test]
fn every_unauthenticated_handler_is_public_on_purpose() {
    let open = ids(scan().iter().filter(|h| !h.takes_auth));
    let listed: BTreeSet<String> = PUBLIC.iter().map(|(f, n, _)| format!("{f}::{n}")).collect();

    let unlisted = &open - &listed;
    assert!(
        unlisted.is_empty(),
        "these answer without a credential but aren't in PUBLIC — gate them, or list them with \
         the reason they're public: {unlisted:?}"
    );
    let stale = &listed - &open;
    assert!(
        stale.is_empty(),
        "these are in PUBLIC but now take a credential: {stale:?}"
    );
    assert!(PUBLIC.iter().all(|(_, _, why)| !why.trim().is_empty()));
}
