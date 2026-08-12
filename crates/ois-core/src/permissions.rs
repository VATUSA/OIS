//! Path-based permission primitives, ported from osmium's `auth/acl.rs` (the pure,
//! DB-free parts). A permission is a dotted string `segments.action`, e.g.
//! `events.items.create`. Nothing is implied by role name — every capability is an
//! explicit granted permission.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

/// The fixed verb set a permission can carry. Kept small and explicit so the
/// access-editor UI and the `RequirePermission<P>` extractor share one vocabulary.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Read,
    Create,
    Update,
    Delete,
    Publish,
    Assign,
    Decide,
    Request,
    Approve,
    Deny,
    /// Claim an open request (e.g. an ACE support request). OIS addition.
    Claim,
}

impl PermissionAction {
    pub fn from_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Self::Read),
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            "publish" => Some(Self::Publish),
            "assign" => Some(Self::Assign),
            "decide" => Some(Self::Decide),
            "request" => Some(Self::Request),
            "approve" => Some(Self::Approve),
            "deny" => Some(Self::Deny),
            "claim" => Some(Self::Claim),
            _ => None,
        }
    }

    pub fn as_value(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Publish => "publish",
            Self::Assign => "assign",
            Self::Decide => "decide",
            Self::Request => "request",
            Self::Approve => "approve",
            Self::Deny => "deny",
            Self::Claim => "claim",
        }
    }
}

/// A fully-qualified permission: ordered segments plus a terminal action.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema)]
pub struct PermissionPath {
    pub segments: Vec<String>,
    pub action: PermissionAction,
}

impl PermissionPath {
    pub fn from_segments<const N: usize>(segments: [&str; N], action: PermissionAction) -> Self {
        Self {
            segments: segments.iter().map(|s| (*s).to_string()).collect(),
            action,
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        let parts: Vec<_> = value.trim().split('.').collect();
        if parts.len() < 2 {
            return None;
        }
        let action = PermissionAction::from_value(parts.last()?)?;
        let segments = parts[..parts.len() - 1]
            .iter()
            .map(|segment| {
                let segment = segment.trim();
                (!segment.is_empty() && is_valid_permission_segment(segment))
                    .then_some(segment.to_string())
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { segments, action })
    }

    pub fn as_db_value(&self) -> String {
        let mut parts = self.segments.clone();
        parts.push(self.action.as_value().to_string());
        parts.join(".")
    }
}

/// Segments are lowercase snake identifiers. Note a segment is either a leaf
/// (action array) or a parent (further segments) in the JSON tree, never both —
/// see osmium migration 0048 / the `feedback.items_self` note.
pub fn is_valid_permission_segment(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

/// Collapse a flat permission list into the nested JSON the access-editor renders.
pub fn permission_tree_from_paths(permissions: &[PermissionPath]) -> Value {
    let mut root = Map::new();
    for permission in permissions {
        insert_permission_path(&mut root, permission);
    }
    Value::Object(root)
}

/// Inverse of [`permission_tree_from_paths`]: validate + flatten an edited tree back
/// to sorted `segments.action` strings. Returns `Err(())` on a malformed shape.
pub fn normalize_permission_tree(value: &Value) -> Result<Vec<String>, ()> {
    let Value::Object(root) = value else {
        return Err(());
    };
    let mut normalized = BTreeSet::new();
    collect_permission_tree(root, &mut Vec::new(), &mut normalized)?;
    if normalized.is_empty() {
        return Err(());
    }
    Ok(normalized.into_iter().collect())
}

fn insert_permission_path(root: &mut Map<String, Value>, permission: &PermissionPath) {
    if permission.segments.is_empty() {
        return;
    }
    let mut node = root;
    for segment in &permission.segments[..permission.segments.len() - 1] {
        let entry = node
            .entry(segment.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(child) = entry else {
            return;
        };
        node = child;
    }
    let leaf = permission.segments.last().cloned().unwrap_or_default();
    let entry = node.entry(leaf).or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(actions) = entry else {
        return;
    };
    let action = permission.action.as_value();
    if !actions.iter().any(|v| v.as_str() == Some(action)) {
        actions.push(Value::String(action.to_string()));
        actions.sort_by(|l, r| l.as_str().cmp(&r.as_str()));
    }
}

fn collect_permission_tree(
    node: &Map<String, Value>,
    path: &mut Vec<String>,
    normalized: &mut BTreeSet<String>,
) -> Result<(), ()> {
    for (key, value) in node {
        if !is_valid_permission_segment(key) {
            return Err(());
        }
        match value {
            Value::Object(child) => {
                path.push(key.clone());
                collect_permission_tree(child, path, normalized)?;
                path.pop();
            }
            Value::Array(actions) => {
                if actions.is_empty() {
                    return Err(());
                }
                let mut action_set = BTreeSet::new();
                for action_value in actions {
                    let action_name = action_value.as_str().ok_or(())?;
                    let action = PermissionAction::from_value(action_name).ok_or(())?;
                    action_set.insert(action.as_value().to_string());
                }
                let mut segments = path.clone();
                segments.push(key.clone());
                for action in action_set {
                    normalized.insert(format!("{}.{}", segments.join("."), action));
                }
            }
            _ => return Err(()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_and_renders_db_values() {
        let p = PermissionPath::from_db_value("events.items.create").unwrap();
        assert_eq!(
            p,
            PermissionPath::from_segments(["events", "items"], PermissionAction::Create)
        );
        assert_eq!(p.as_db_value(), "events.items.create");
        assert!(PermissionPath::from_db_value("nope").is_none());
    }

    #[test]
    fn builds_and_normalizes_tree() {
        let tree = permission_tree_from_paths(&[
            PermissionPath::from_segments(["access", "users"], PermissionAction::Update),
            PermissionPath::from_segments(["access", "users"], PermissionAction::Read),
            PermissionPath::from_segments(["ace", "requests"], PermissionAction::Claim),
        ]);
        assert_eq!(
            tree,
            json!({
                "ace": { "requests": ["claim"] },
                "access": { "users": ["read", "update"] }
            })
        );
        let flat = normalize_permission_tree(&tree).unwrap();
        assert_eq!(
            flat,
            vec![
                "access.users.read".to_string(),
                "access.users.update".to_string(),
                "ace.requests.claim".to_string(),
            ]
        );
    }

    #[test]
    fn rejects_bad_shape() {
        assert!(normalize_permission_tree(&json!({ "events": "read" })).is_err());
    }
}
