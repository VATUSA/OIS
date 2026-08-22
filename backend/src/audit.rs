//! Audit middleware — records every successful state-changing request (POST/PUT/PATCH/DELETE)
//! to the audit log, so any create/update/delete/publish across the site is captured without
//! each handler opting in. Runs after `resolve_current_user` (so the actor is known) and is
//! best-effort: an audit-write failure never fails the user's request. The access editor keeps
//! its own richer before/after entry, so its route is excluded here to avoid a duplicate.

use axum::{
    extract::{MatchedPath, Request, State},
    middleware::Next,
    response::Response,
};
use http::Method;

use crate::{
    auth::context::{CurrentApiKey, CurrentServiceAccount, CurrentUser},
    repos::audit as audit_repo,
    state::AppState,
};

/// Action verbs that show up as a trailing static path segment (e.g. `.../{id}/publish`).
const KNOWN_VERBS: &[&str] = &[
    "publish", "cancel", "activate", "compress", "rotate", "disable", "refresh", "reorder",
    "release", "share", "copy",
];

/// resource_types whose handlers already write their own richer audit entry (so the generic
/// middleware entry would just be a duplicate). Everything else is logged. `admin.users` is the
/// access editor (`PUT /admin/users/{id}/access`), which records its own before/after entry.
fn is_excluded(resource_type: &str) -> bool {
    // `flow.resolve-routes` is a read (POST only because it takes a list body), not a mutation.
    // `api-keys` / `admin.api-keys` handlers write their own richer before/after audit entries.
    matches!(
        resource_type,
        "admin.users" | "flow.resolve-routes" | "api-keys" | "admin.api-keys"
    )
}

/// Fallback action for a bare create/update/delete with no verb segment.
fn method_action(method: &Method) -> Option<&'static str> {
    match *method {
        Method::POST => Some("create"),
        Method::PUT | Method::PATCH => Some("update"),
        Method::DELETE => Some("delete"),
        _ => None,
    }
}

fn is_param(seg: &str) -> bool {
    seg.starts_with('{')
}

/// Derive `(action, resource_type, resource_id)` from the matched route template + real path.
/// e.g. `PUT /api/v1/tmu/programs/{icao}` on `.../KSFO` → `("update", "tmu.programs", "KSFO")`;
/// `POST /api/v1/tmu/gdp/{id}/publish` → `("publish", "tmu.gdp", "<id>")`.
fn derive(
    method: &Method,
    template: &str,
    actual: &str,
) -> Option<(String, String, Option<String>)> {
    let segs = |s: &str| -> Vec<String> {
        s.trim_start_matches('/')
            .split('/')
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut t = segs(template);
    let mut p = segs(actual);
    // Drop the shared "api/v1" prefix from both.
    if t.first().map(String::as_str) == Some("api") && t.get(1).map(String::as_str) == Some("v1") {
        t.drain(..2);
        if p.len() >= 2 {
            p.drain(..2);
        }
    }
    if t.is_empty() {
        return None;
    }

    let last_idx = t.len() - 1;
    let last = t[last_idx].as_str();
    let has_param_before_last = t[..last_idx].iter().any(|s| is_param(s));

    let (action, verb_is_last) = if is_param(last) {
        (method_action(method)?.to_string(), false)
    } else if KNOWN_VERBS.contains(&last) || has_param_before_last {
        (last.to_string(), true)
    } else {
        (method_action(method)?.to_string(), false)
    };

    let resource_type = t
        .iter()
        .enumerate()
        .filter(|(i, s)| !(is_param(s) || verb_is_last && *i == last_idx))
        .map(|(_, s)| s.as_str())
        .collect::<Vec<_>>()
        .join(".");
    if resource_type.is_empty() {
        return None;
    }

    let resource_id = t
        .iter()
        .enumerate()
        .rfind(|(_, s)| is_param(s))
        .and_then(|(i, _)| p.get(i).cloned());

    Some((action, resource_type, resource_id))
}

/// Realtime nudge topic for the broader TMU boards (GDP / TMI / ground stops / rate programs), keyed
/// off the matched route. FCA releases, FCA edits, and CFRs publish precisely from their own handlers,
/// so they're intentionally excluded here to avoid a double nudge.
fn tmu_realtime_topic(path: &str) -> Option<&'static str> {
    use crate::realtime::topic;
    if path.starts_with("/api/v1/tmu/gdp") {
        Some(topic::GDP)
    } else if path.starts_with("/api/v1/tmu/tmis") {
        Some(topic::TMI)
    } else if path.starts_with("/api/v1/tmu/ground-stops") {
        Some(topic::GROUND_STOP)
    } else if path.starts_with("/api/v1/tmu/programs") {
        Some(topic::PROGRAM)
    } else {
        None
    }
}

/// Middleware: log a successful mutation to the audit trail.
pub async fn audit_mutations(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let is_mutation = method_action(&method).is_some();

    // Capture what we need before the request is consumed by the handler.
    let template = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string());
    let actual = request.uri().path().to_string();
    let user = request
        .extensions()
        .get::<Option<CurrentUser>>()
        .cloned()
        .flatten();
    let api_key = request
        .extensions()
        .get::<Option<CurrentApiKey>>()
        .cloned()
        .flatten();
    let service_account = request
        .extensions()
        .get::<Option<CurrentServiceAccount>>()
        .cloned()
        .flatten();
    let ip = audit_repo::client_ip(request.headers());

    let response = next.run(request).await;

    if is_mutation && response.status().is_success() {
        if let Some(topic) = tmu_realtime_topic(template.as_deref().unwrap_or(&actual)) {
            state.publish(topic);
        }
        let actor = Actor {
            user,
            api_key,
            service_account,
        };
        record(&state, &method, template.as_deref(), &actual, actor, ip).await;
    }
    response
}

/// Whichever principal authenticated the request — a request carries at most one.
struct Actor {
    user: Option<CurrentUser>,
    api_key: Option<CurrentApiKey>,
    service_account: Option<CurrentServiceAccount>,
}

async fn record(
    state: &AppState,
    method: &Method,
    template: Option<&str>,
    actual: &str,
    actor: Actor,
    ip: Option<String>,
) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    let Some((action, resource_type, resource_id)) =
        derive(method, template.unwrap_or(actual), actual)
    else {
        return;
    };
    if is_excluded(&resource_type) {
        return;
    }
    // Resolve (creating if needed) the actor for whichever principal made the request — user,
    // api key, or service account. An unauthenticated mutation has no actor and isn't logged.
    // A resolution failure still logs with a null actor so the action is never silently dropped.
    let actor_id = if let Some(user) = actor.user.as_ref() {
        audit_repo::resolve_user_actor_id(pool, &user.id, &user.display_name)
            .await
            .ok()
            .flatten()
    } else if let Some(key) = actor.api_key.as_ref() {
        audit_repo::resolve_api_key_actor_id(
            pool,
            &key.id,
            &format!("{} ({})", key.name, key.prefix),
        )
        .await
        .ok()
        .flatten()
    } else if let Some(sa) = actor.service_account.as_ref() {
        audit_repo::resolve_service_account_actor_id(pool, &sa.id, &sa.name)
            .await
            .ok()
            .flatten()
    } else {
        return; // unauthenticated mutation — nothing to attribute
    };
    // Best-effort: never fail the user's request over an audit write.
    let _ = audit_repo::record_audit(
        pool,
        audit_repo::AuditEntry {
            actor_id,
            action,
            resource_type,
            resource_id,
            artcc_id: None,
            reason: None,
            before_state: None,
            after_state: None,
            ip_address: ip,
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: Method, tmpl: &str, actual: &str) -> (String, String, Option<String>) {
        derive(&m, tmpl, actual).unwrap()
    }

    #[test]
    fn update_and_delete_by_id() {
        assert_eq!(
            d(
                Method::PUT,
                "/api/v1/tmu/programs/{icao}",
                "/api/v1/tmu/programs/KSFO"
            ),
            ("update".into(), "tmu.programs".into(), Some("KSFO".into())),
        );
        assert_eq!(
            d(
                Method::DELETE,
                "/api/v1/tmu/programs/{icao}",
                "/api/v1/tmu/programs/KSFO"
            ),
            ("delete".into(), "tmu.programs".into(), Some("KSFO".into())),
        );
    }

    #[test]
    fn create_on_collection() {
        assert_eq!(
            d(Method::POST, "/api/v1/tmu/gdp", "/api/v1/tmu/gdp"),
            ("create".into(), "tmu.gdp".into(), None),
        );
    }

    #[test]
    fn trailing_verb_is_the_action() {
        assert_eq!(
            d(
                Method::POST,
                "/api/v1/tmu/gdp/{id}/publish",
                "/api/v1/tmu/gdp/abc/publish"
            ),
            ("publish".into(), "tmu.gdp".into(), Some("abc".into())),
        );
        // Collection-level verb with no id param.
        assert_eq!(
            d(
                Method::POST,
                "/api/v1/flow/data/refresh",
                "/api/v1/flow/data/refresh"
            ),
            ("refresh".into(), "flow.data".into(), None),
        );
    }

    #[test]
    fn nested_resource_uses_last_param_as_id() {
        assert_eq!(
            d(
                Method::POST,
                "/api/v1/tmu/gdp/{id}/slots/{callsign}",
                "/api/v1/tmu/gdp/abc/slots/AAL1",
            ),
            ("create".into(), "tmu.gdp.slots".into(), Some("AAL1".into())),
        );
    }

    #[test]
    fn access_editor_is_excluded() {
        // PUT /admin/users/{id}/access derives to "admin.users"; the handler logs its own
        // richer before/after entry, so the middleware skips it to avoid a duplicate.
        assert_eq!(
            d(
                Method::PUT,
                "/api/v1/admin/users/{id}/access",
                "/api/v1/admin/users/U1/access"
            ),
            ("access".into(), "admin.users".into(), Some("U1".into())),
        );
        assert!(is_excluded("admin.users"));
        assert!(!is_excluded("tmu.programs"));
        // API-key routes write their own richer entries, so the generic middleware skips them.
        assert!(is_excluded("api-keys"));
        assert!(is_excluded("admin.api-keys"));
    }
}
