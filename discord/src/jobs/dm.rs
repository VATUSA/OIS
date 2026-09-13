use std::sync::Arc;

use serde_json::Value;
use serenity::all::{CreateMessage, Http, UserId};

use crate::util::str_field;

/// The DM body: a claim confirmation, plus either the facility's configured documents or a note
/// that none exist yet. A reminder (`reminder: Some("24h" | "6h")`) is a short line only — the
/// documents were already sent in the original confirmation, no need to re-list them.
fn build_dm_content(
    event_title: &str,
    position: Option<&str>,
    documents: &[Value],
    reminder: Option<&str>,
) -> String {
    if let Some(when) = reminder {
        return match position {
            Some(p) if !p.is_empty() => {
                format!(
                    "⏰ Reminder: you're confirmed to work **{p}** at **{event_title}** in about {when}."
                )
            }
            _ => format!("⏰ Reminder: you're confirmed for **{event_title}** in about {when}."),
        };
    }
    let confirmation = match position {
        Some(p) if !p.is_empty() => {
            format!("✅ You're confirmed to work **{p}** at **{event_title}**.")
        }
        _ => format!("✅ You're confirmed for **{event_title}**."),
    };
    if documents.is_empty() {
        return format!("{confirmation}\n\nNo documents configured for this facility yet.");
    }
    let list: String = documents
        .iter()
        .filter_map(|d| {
            let title = d.get("title").and_then(Value::as_str)?;
            let url = d.get("url").and_then(Value::as_str)?;
            Some(format!("• [{title}]({url})"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{confirmation}\n\n**Documents:**\n{list}")
}

pub(crate) async fn send_claim_dm(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let discord_user_id: u64 = str_field(p, "discord_user_id")
        .ok_or("missing discord_user_id")?
        .parse()
        .map_err(|_| "discord_user_id not a snowflake".to_string())?;
    let event_title = str_field(p, "event_title").unwrap_or("your event");
    let position = str_field(p, "position");
    let documents = p
        .get("documents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let reminder = str_field(p, "reminder");

    let content = build_dm_content(event_title, position, &documents, reminder);
    UserId::new(discord_user_id)
        .dm(http, CreateMessage::new().content(content))
        .await
        .map_err(|e| format!("dm failed: {e}"))?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_dm_content_lists_documents_when_present() {
        let docs = vec![
            json!({"title": "ZDC SOP", "url": "https://example.com/sop"}),
            json!({"title": "ZDC LOA", "url": "https://example.com/loa"}),
        ];
        let out = build_dm_content("Fall Fly-In", Some("DCA_APP"), &docs, None);
        assert_eq!(
            out,
            "✅ You're confirmed to work **DCA_APP** at **Fall Fly-In**.\n\n\
             **Documents:**\n\
             • [ZDC SOP](https://example.com/sop)\n\
             • [ZDC LOA](https://example.com/loa)"
        );
    }

    #[test]
    fn build_dm_content_notes_no_documents() {
        let out = build_dm_content("Fall Fly-In", Some("DCA_APP"), &[], None);
        assert_eq!(
            out,
            "✅ You're confirmed to work **DCA_APP** at **Fall Fly-In**.\n\n\
             No documents configured for this facility yet."
        );
    }

    #[test]
    fn build_dm_content_without_a_position() {
        let out = build_dm_content("Fall Fly-In", None, &[], None);
        assert_eq!(
            out,
            "✅ You're confirmed for **Fall Fly-In**.\n\n\
             No documents configured for this facility yet."
        );
    }

    #[test]
    fn build_dm_content_reminder_is_short_and_skips_documents() {
        let docs = vec![json!({"title": "ZDC SOP", "url": "https://example.com/sop"})];
        let out = build_dm_content("Fall Fly-In", Some("DCA_APP"), &docs, Some("24h"));
        assert_eq!(
            out,
            "⏰ Reminder: you're confirmed to work **DCA_APP** at **Fall Fly-In** in about 24h."
        );
    }

    #[test]
    fn build_dm_content_reminder_without_a_position() {
        let out = build_dm_content("Fall Fly-In", None, &[], Some("6h"));
        assert_eq!(
            out,
            "⏰ Reminder: you're confirmed for **Fall Fly-In** in about 6h."
        );
    }
}
