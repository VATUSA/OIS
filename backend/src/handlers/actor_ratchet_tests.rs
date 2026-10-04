//! VATUSA/OIS#583 AC4, as a ratchet: every handler that still identifies its caller through
//! `Extension<Option<CurrentUser>>` is listed here, with its intent. That extractor is session- and
//! desktop-only, so a machine credential that clears `RequirePermission` is refused on the next line.
//!
//! - `UserOnly(reason)`: about a person by nature, and must stay that way. Every other handler takes
//!   [`crate::auth::principal::Actor`], which admits a user, an API key or a service account (#607).
//!
//! A handler that takes `CurrentUser` and is not listed fails this test, so a new machine-blocked
//! write can't land by accident. So does a listed one that no longer does, so the list can't rot.

use std::collections::BTreeSet;

#[derive(Debug)]
enum Intent {
    UserOnly(&'static str),
}
use Intent::UserOnly;

#[rustfmt::skip]
const CURRENT_USER_HANDLERS: &[(&str, &str, Intent)] = &[
    ("access", "get_self_access", UserOnly("the caller's own access summary")),
    ("access", "update_user_access", UserOnly("editing a person's access is a human act; a key's denylist covers only api_keys.*, so a machine here could grant access")),
    ("access", "resync_user_vatusa", UserOnly("re-attaching a person to VATUSA role sync changes their access, like update_user_access; a machine here could grant access")),
    ("access", "create_group", UserOnly("editing what a group grants is a human act, like update_user_access; a machine here could grant access")),
    ("access", "update_group", UserOnly("editing what a group grants is a human act, like update_user_access; a machine here could grant access")),
    ("access", "delete_group", UserOnly("editing what a group grants is a human act, like update_user_access; a machine here could grant access")),
    ("access", "add_group_member", UserOnly("granting group membership grants the group's access, like update_user_access; a machine here could grant access")),
    ("access", "remove_group_member", UserOnly("granting group membership grants the group's access, like update_user_access; a machine here could grant access")),
    ("access", "create_vatusa_role_mapping", UserOnly("mapping a VATUSA role to a group grants that group to every synced holder of the role, like group membership; a machine here could grant access")),
    ("access", "delete_vatusa_role_mapping", UserOnly("mapping a VATUSA role to a group grants that group to every synced holder of the role, like group membership; a machine here could grant access")),
    ("ace", "my_ace_claims", UserOnly("a controller's own ACE claims")),
    ("ace", "create_request", UserOnly("an ACE request is a controller asking for support; its requester is a person, named in the UI and on Discord")),
    ("ace", "claim_request", UserOnly("a controller claims an ACE request for themselves")),
    ("ace", "release_claim", UserOnly("a controller gives back their own claim")),
    ("api_keys", "list_my_keys", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "grantable_permissions", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "create_key", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "get_my_key", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "rotate_key", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "set_key_permissions", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "disable_my_key", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "delete_my_key", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "key_audit", UserOnly("a person's own API keys; a key must never manage keys")),
    ("api_keys", "admin_disable_key", UserOnly("managing API keys is a human act; keys can never hold api_keys.*")),
    ("api_keys", "admin_delete_key", UserOnly("managing API keys is a human act; keys can never hold api_keys.*")),
    ("auth", "me", UserOnly("the signed-in person")),
    ("dashboards", "list_dashboards", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "create_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "get_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "update_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "delete_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "share_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "unshare_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "get_shared_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "copy_shared_dashboard", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "create_collection", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "rename_collection", UserOnly("personal dashboards, owned by a person")),
    ("dashboards", "delete_collection", UserOnly("personal dashboards, owned by a person")),
    ("diagnostics", "upload_report", UserOnly("a person sends it from their own desktop app")),
    ("events", "generate_tier1", UserOnly("an ACE request is a controller asking for support; its requester is a person, named in the UI and on Discord")),
    ("flow", "my_flight", UserOnly("the signed-in pilot's own flight")),
    ("integration", "get_my_discord", UserOnly("the caller's own linked Discord account")),
    ("preferences", "get_preferences", UserOnly("per-person UI preferences")),
    ("preferences", "put_preferences", UserOnly("per-person UI preferences")),
    ("service_accounts", "rotate_service_account", UserOnly("a person's own authority caps the grant (#584)")),
    ("service_accounts", "set_service_account_roles", UserOnly("a person's own authority caps the grant (#584)")),
    ("service_accounts", "grantable_service_account_permissions", UserOnly("a person's own authority caps the grant (#584)")),
    ("service_accounts", "set_service_account_permissions", UserOnly("a person's own authority caps the grant (#584)")),
];

/// `(file stem, fn)` for every `pub async fn` in `handlers/*.rs` whose parameters take
/// `Extension<Option<CurrentUser>>`. Read from disk, so a file not yet in `mod.rs` is scanned too.
fn scan(dir: &std::path::Path) -> BTreeSet<(String, String)> {
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(&path).unwrap();
        for (at, _) in src.match_indices("pub async fn ") {
            let rest = &src[at + "pub async fn ".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            // The parameter list: from the first `(` to its matching `)`.
            let Some(open) = rest.find('(') else { continue };
            let mut depth = 0usize;
            let mut close = open;
            for (i, c) in rest[open..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = open + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let params: String = rest[open..=close].split_whitespace().collect();
            if params.contains(&["Extension<Option<", "CurrentUser>>"].concat()) {
                found.insert((stem.clone(), name));
            }
        }
    }
    found
}

fn handlers_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/handlers")
}

#[test]
fn every_current_user_handler_is_listed_with_its_intent() {
    let listed: BTreeSet<(String, String)> = CURRENT_USER_HANDLERS
        .iter()
        .map(|(f, n, _)| (f.to_string(), n.to_string()))
        .collect();
    assert_eq!(
        listed.len(),
        CURRENT_USER_HANDLERS.len(),
        "a handler is listed twice"
    );
    let found = scan(&handlers_dir());

    let unlisted: Vec<_> = found.difference(&listed).collect();
    assert!(
        unlisted.is_empty(),
        "these take `CurrentUser`, so a service account or API key can never call them. Take \
         `Actor` instead (auth/principal.rs), or list them here as `UserOnly(\"why\")`: {unlisted:?}"
    );
    let stale: Vec<_> = listed.difference(&found).collect();
    assert!(
        stale.is_empty(),
        "listed but no longer take `CurrentUser`; remove them: {stale:?}"
    );
}

#[test]
fn every_user_only_entry_says_why() {
    for (file, name, UserOnly(reason)) in CURRENT_USER_HANDLERS {
        assert!(
            !reason.trim().is_empty(),
            "{file}::{name} is user-only with no reason"
        );
    }
}

/// The release path is migrated and must stay off the list (#583's whole point).
#[test]
fn the_release_path_is_not_on_the_list() {
    for (file, name) in [
        ("flow", "mark_release"),
        ("flow", "clear_release"),
        ("flow", "swap_releases"),
        ("feed", "issue_cfr"),
        ("feed", "release_cfr"),
    ] {
        assert!(
            !CURRENT_USER_HANDLERS
                .iter()
                .any(|(f, n, _)| *f == file && *n == name),
            "{file}::{name} is on the release path and must admit a machine"
        );
    }
}
