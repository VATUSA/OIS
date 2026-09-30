use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{
    ButtonStyle, CreateActionRow, CreateAllowedMentions, CreateButton, CreateMessage, Http,
};

use crate::util::{TMI_STRUCTURED_PREFIX, channel, str_field};

/// Wraps an NTML row in a bare code block.
///
/// The real NTML channel is monospace, column-aligned rows — proportional text loses the alignment
/// that makes a log of restrictions scannable. A bare block rather than a language-tagged one or an
/// embed: it is closest to what the channel actually carries (#436).
fn code_block(line: &str) -> String {
    format!("```\n{line}\n```")
}

pub(crate) async fn post_tmi(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    // The backend assembles the whole row — log stamp, restriction, valid window, REQ:PROV — so the
    // bot never improvises NTML's shape. It used to build a bold `**N90 → ZNY**` header here
    // precisely because nothing owned the complete line (#436).
    let line = str_field(p, "ntml").unwrap_or("(no restriction text)");
    let content = code_block(line);

    let message = build_message(&content, str_field(p, "tmi_id"));

    let message = channel
        .send_message(http, message)
        .await
        .map_err(|e| format!("send_message failed: {e}"))?;
    Ok(Some(json!({ "message_id": message.id.get().to_string() })))
}

/// The message as it will be sent. Split out from [`post_tmi`] so the hardening below can actually
/// be asserted — it is otherwise only reachable through a live Discord connection.
fn build_message(content: &str, tmi_id: Option<&str>) -> CreateMessage {
    let mut message = CreateMessage::new()
        .content(content)
        // The restriction is free-form text a TMU controller typed (`create_tmi`/`update_tmi` apply
        // no mention-syntax filtering); unlike the embed this replaces — which Discord never parses
        // for mentions — plain message content DOES get parsed by default when this is omitted,
        // turning an accidental "@everyone" in a TMI into a real mass-ping of the channel.
        .allowed_mentions(CreateAllowedMentions::new());
    if let Some(tmi_id) = tmi_id {
        let button = CreateButton::new(format!("{TMI_STRUCTURED_PREFIX}{tmi_id}"))
            .label("View structured")
            .style(ButtonStyle::Secondary);
        message = message.components(vec![CreateActionRow::Buttons(vec![button])]);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The NTML channel is monospace, column-aligned rows; proportional text loses the alignment
    /// that makes a log of restrictions scannable (#436).
    #[test]
    fn wraps_the_row_in_a_bare_code_block() {
        assert_eq!(
            code_block("14/1442 JFK arrivals via CAMRN 20MIT 2015-2315 N90:ZNY"),
            "```\n14/1442 JFK arrivals via CAMRN 20MIT 2015-2315 N90:ZNY\n```"
        );
    }

    /// A TMI is free text a controller typed, and `create_tmi`/`update_tmi` apply no mention
    /// filtering. Plain message content IS parsed for mentions by default, so omitting this turns an
    /// accidental `@everyone` into a real mass-ping. Asserted on the serialized payload, because the
    /// builder is otherwise only observable through a live connection.
    #[test]
    fn an_at_everyone_in_a_tmi_cannot_ping_the_channel() {
        let message = build_message(&code_block("14/1442 @everyone STOP ZNY"), None);
        let json = serde_json::to_value(&message).expect("serialize");

        let mentions = json
            .get("allowed_mentions")
            .expect("allowed_mentions must be set, or Discord parses the content");
        // An empty `parse` is what makes it inert: no roles, no users, no @everyone.
        let parse = mentions.get("parse").and_then(|p| p.as_array());
        assert_eq!(parse.map(|p| p.len()), Some(0), "got {mentions:?}");

        // And the content really does still contain the text — otherwise this passes vacuously.
        assert!(
            json.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("@everyone")),
            "the test should be checking a message that actually carries the mention"
        );
    }

    /// The "View structured" button is attached only when there's a TMI to look up.
    #[test]
    fn the_structured_button_is_attached_only_with_a_tmi_id() {
        let with = serde_json::to_value(build_message("x", Some("tmi-7"))).unwrap();
        let without = serde_json::to_value(build_message("x", None)).unwrap();

        let has_components = |v: &serde_json::Value| {
            v.get("components")
                .and_then(|c| c.as_array())
                .is_some_and(|c| !c.is_empty())
        };
        assert!(has_components(&with));
        assert!(!has_components(&without));
    }
}
