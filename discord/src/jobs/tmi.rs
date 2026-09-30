use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{
    ButtonStyle, CreateActionRow, CreateAllowedMentions, CreateButton, CreateMessage, Http,
};

use crate::util::{TMI_STRUCTURED_PREFIX, channel, str_field};

/// The assembled NTML row from a job payload.
///
/// A hard error, not a placeholder. The row is the backend's to produce (`tmi::ntml_line` /
/// `ntml_cancel_line`); if it is absent the payload is not one this bot understands — a key that moved,
/// or a job enqueued by a backend from before the payload carried `ntml`, still pending or mid-retry
/// across a deploy (`ack_job` retries up to `MAX_ATTEMPTS`). Posting "(no restriction text)" instead
/// *succeeded*, so `ack_job` marked it `succeeded`: a wrong row in the NTML channel, indistinguishable
/// from a right one. Failing lets it retry and then park as `failed` with a greppable reason.
///
/// Split out for the same reason as [`build_message`] and `jobs::route` — `post_tmi` needs a live
/// `Http`, so the decision is only assertable from here (#436 review).
fn ntml_row(p: &Value) -> Result<&str, String> {
    str_field(p, "ntml").ok_or_else(|| "payload carries no assembled ntml row".to_string())
}

/// Flattens an NTML row to something that cannot escape the block it is about to be wrapped in.
///
/// The restriction is free text a TMU controller typed — `create_tmi`/`update_tmi` filter no
/// characters, and `encode`'s `TXT` arm passes the text through verbatim. A backtick run closes the
/// fence early, so the row loses the monospace alignment that is the whole point and everything after
/// it renders as live Discord markdown, links included, posted by the bot into the NTML channel. A
/// newline splits one row into two. Neither has any meaning in NTML grammar, so both are replaced
/// rather than escaped (#436 review).
fn flatten(line: &str) -> String {
    line.chars()
        .map(|c| match c {
            '`' => '\'',
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

/// Wraps an NTML row in a bare code block.
///
/// The real NTML channel is monospace, column-aligned rows — proportional text loses the alignment
/// that makes a log of restrictions scannable. A bare block rather than a language-tagged one or an
/// embed: it is closest to what the channel actually carries (#436).
fn code_block(line: &str) -> String {
    format!("```\n{}\n```", flatten(line))
}

pub(crate) async fn post_tmi(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    // The backend assembles the whole row — log stamp, restriction, valid window, REQ:PROV — so the
    // bot never improvises NTML's shape. It used to build a bold `**N90 → ZNY**` header here
    // precisely because nothing owned the complete line (#436).
    let line = ntml_row(p)?;
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

    /// A payload without the assembled row must fail the job, not post a placeholder. It used to be
    /// `unwrap_or("(no restriction text)")`, which Discord accepted — so `ack_job` recorded a
    /// `succeeded` post of a row that says nothing, and nobody would know (#436 review).
    #[test]
    fn a_payload_without_the_assembled_row_fails_rather_than_posting_a_placeholder() {
        assert!(ntml_row(&json!({ "channel_id": "1" })).is_err());
        // A renamed key is the same case, and is how this would break silently.
        assert!(ntml_row(&json!({ "ntml_row": "14/1442 STOP ZNY" })).is_err());
        // An empty string is not a row either — `str_field` already filters it.
        assert!(ntml_row(&json!({ "ntml": "" })).is_err());

        assert_eq!(
            ntml_row(&json!({ "ntml": "14/1442 STOP ZNY" })).unwrap(),
            "14/1442 STOP ZNY"
        );
    }

    /// A restriction is free text and nothing filters it, so a backtick run in one would otherwise
    /// close the fence early: the row stops being monospace and whatever follows — a link, say —
    /// renders live in the channel. Probed before the fix: `14/1442 STOP ZNY ``` [click me](…)`
    /// produced three fences (#436 review).
    #[test]
    fn a_restriction_cannot_break_out_of_the_code_block() {
        let content = code_block("14/1442 STOP ZNY ``` [click me](https://evil.example)");

        assert_eq!(
            content.matches("```").count(),
            2,
            "one fence open, one closed, and nothing of the row's own: {content}"
        );
        // Check the row itself, not the fences that legitimately contain backticks.
        let row = content.lines().nth(1).expect("fence, row, fence");
        assert!(!row.contains('`'), "{row}");
        // And it is still legible — the characters are replaced, not dropped.
        assert!(row.starts_with("14/1442 STOP ZNY"), "{row}");
        assert!(row.contains("click me"), "{row}");
    }

    /// A newline would split one NTML row across two lines of the block, which is what makes the log
    /// scannable by row (#436 review).
    #[test]
    fn a_newline_in_a_restriction_does_not_split_the_row() {
        let content = code_block("14/1442 STOP ZNY\nnot a second row");

        assert_eq!(
            content.lines().count(),
            3,
            "fence, one row, fence: {content}"
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
