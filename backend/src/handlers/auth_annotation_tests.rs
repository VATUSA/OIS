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
//!
//! #587 adds the document's other half, its security requirements:
//!
//! - **(d)** a handler that takes a credential declares, as alternatives, exactly the schemes its
//!   extractors accept (see [`expected_schemes`]) — each scoped to its `RequirePermission` marker's
//!   permission, or `[]` when it checks identity only — so a path's scope is the grant an integrator
//!   must request, and a credential kind is never promised a path that would 401 it;
//! - **(e)** a handler that takes no credential declares no `security`.

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
    ("facility_map", "get_config", "the rules and colours the public facility map renders with; the optional identity only sets `editable`"),
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

/// Public handlers that also read an **optional** caller identity, only to show a signed-in caller
/// more. An anonymous request still succeeds, so each stays in [`PUBLIC`], and the scan would otherwise
/// misread its identity extractor as a gate.
const PUBLIC_WITH_OPTIONAL_IDENTITY: &[(&str, &str)] = &[
    // Hidden (unpublished-event or deleted) FCAs are served to planners / signed-in callers only (#586).
    ("flow", "fca_traffic"),
    // Serves the same config to everyone; the identity only decides the `editable` hint (#586).
    ("facility_map", "get_config"),
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
    "Option<Actor>",
];

struct Handler {
    file: String,
    name: String,
    advertises_401: bool,
    takes_permission: bool,
    takes_auth: bool,
    /// The `M` of a `RequirePermission<M>` parameter.
    marker: Option<String>,
    /// The annotation's `security(...)` argument, whitespace removed.
    security: Option<String>,
    /// The parameter list, whitespace removed.
    params: String,
    /// The source from the parameter list to the next annotation — enough to see how the handler
    /// resolves its caller (`Principal::require` admits fewer credential kinds than `Actor`).
    body: String,
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
        let params_end = balanced(src, params_start + 1);
        let params = squash(&src[params_start..params_end]);
        let body = src[params_end..]
            .find(ANNOTATION)
            .map_or(&src[params_end..], |i| &src[params_end..params_end + i])
            .to_string();
        let marker = params
            .split("RequirePermission<")
            .nth(1)
            .map(|rest| rest.split('>').next().unwrap().to_string());
        let security = annotation.find("security(").map(|i| {
            let open = i + "security(".len();
            annotation[open..balanced(&annotation, open) - 1].to_string()
        });

        out.push(Handler {
            file: file.to_string(),
            name: name.trim().to_string(),
            advertises_401: annotation.contains("status=401"),
            takes_permission: params.contains("RequirePermission<"),
            takes_auth: AUTH_EXTRACTORS.iter().any(|e| params.contains(e)),
            marker,
            security,
            params,
            body,
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
    let optional: BTreeSet<String> = PUBLIC_WITH_OPTIONAL_IDENTITY
        .iter()
        .map(|(f, n)| format!("{f}::{n}"))
        .collect();
    let stale = &(&listed - &open) - &optional;
    assert!(
        stale.is_empty(),
        "these are in PUBLIC but now take a credential: {stale:?}"
    );
    // The exception list can't rot either: each entry is public and really does read an identity.
    let identity_readers = ids(scan().iter().filter(|h| h.takes_auth));
    for id in &optional {
        assert!(
            listed.contains(id),
            "{id} reads an optional identity but isn't in PUBLIC"
        );
        assert!(
            identity_readers.contains(id),
            "{id} no longer reads an identity; drop it from PUBLIC_WITH_OPTIONAL_IDENTITY"
        );
    }
    assert!(PUBLIC.iter().all(|(_, _, why)| !why.trim().is_empty()));
}

/// The security schemes a handler accepts, judged from how it resolves its caller (#587 review):
///
/// | Handler identifies its caller with | Accepts |
/// | --- | --- |
/// | `Actor` (or `Principal::require_any`), or only `RequirePermission` | session, api_key, service_account |
/// | `Principal::require` | session, api_key |
/// | `Extension<Option<CurrentUser>>` / `SessionToken`, or a body that demands the session user | session |
///
/// A body that refuses without a session user (`current_user.as_ref().ok_or(ApiError::Unauthorized)`)
/// is session-only **whatever else it calls**: four handlers did that before `Principal::require`, so
/// an API key was refused on the first line while the spec offered it (#587 review).
fn expected_schemes(h: &Handler) -> &'static [&'static str] {
    const ALL: &[&str] = &["session", "api_key", "service_account"];
    let body: String = h.body.split_whitespace().collect();
    if h.params.contains(":Actor") || h.body.contains("Principal::require_any") {
        ALL
    } else if body.contains("current_user.as_ref().ok_or(ApiError::Unauthorized)") {
        &["session"]
    } else if h.body.contains("Principal::require(") || h.body.contains("Principal::optional(") {
        &["session", "api_key"]
    } else if h.params.contains("CurrentUser") || h.params.contains("SessionToken") {
        &["session"]
    } else {
        ALL
    }
}

#[test]
fn every_gated_handler_declares_the_credentials_and_permission_it_requires() {
    let permissions: std::collections::HashMap<String, String> =
        crate::auth::permissions::marker_permissions()
            .into_iter()
            .collect();
    let handlers = scan();
    let wrong: Vec<String> = handlers
        .iter()
        .filter(|h| h.takes_auth && !optional_identity(h))
        .filter_map(|h| {
            let scope = match &h.marker {
                Some(marker) => format!(
                    r#"["{}"]"#,
                    permissions
                        .get(marker)
                        .unwrap_or_else(|| panic!("{marker} has no permission marker line"))
                ),
                None => "[]".to_string(),
            };
            let expected = expected_schemes(h)
                .iter()
                .map(|scheme| format!(r#"("{scheme}"={scope})"#))
                .collect::<Vec<_>>()
                .join(",");
            (h.security.as_deref() != Some(expected.as_str())).then(|| {
                format!(
                    "{}::{} needs security({expected}), has {:?}",
                    h.file, h.name, h.security
                )
            })
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "a gated path must declare exactly the credentials its handler accepts, scoped to the \
         permission it checks: {wrong:#?}"
    );
    // Both surviving tiers occur, so the rule is exercised rather than collapsing to one answer.
    // The two-kind tier (`Principal::require`) is empty since #607 migrated its handlers to
    // `Actor`; `expected_schemes` keeps the arm, so it returns if such a handler reappears.
    for tier in [3, 1] {
        assert!(
            handlers
                .iter()
                .any(|h| h.takes_auth && expected_schemes(h).len() == tier),
            "no handler accepts exactly {tier} credential kind(s); the scan is misreading handlers"
        );
    }
}

// #587's `a_principal_require_path_does_not_offer_service_accounts` lived here. #607 migrated every
// handler off `Principal::require` — `airport_configs::list_all_airport_configs` among them — so the
// test pinned a premise with no instances, and its assertion would now force the document to
// under-report what an `Actor` handler accepts. `expected_schemes` keeps the `Principal::require`
// arm, so the rule still applies if such a handler reappears.

/// A public handler that reads an optional identity (#586) is still public: it declares no security,
/// or Swagger would ask for a credential the route doesn't need.
fn optional_identity(h: &Handler) -> bool {
    PUBLIC_WITH_OPTIONAL_IDENTITY
        .iter()
        .any(|(file, name)| h.file == *file && h.name == *name)
}

#[test]
fn no_public_handler_declares_security() {
    let claimed = ids(scan()
        .iter()
        .filter(|h| (!h.takes_auth || optional_identity(h)) && h.security.is_some()));
    assert!(
        claimed.is_empty(),
        "these take no credential but declare security, so Swagger would ask for one: {claimed:?}"
    );
}
