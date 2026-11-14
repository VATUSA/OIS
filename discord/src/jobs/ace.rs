use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{
    CreateActionRow, CreateAllowedMentions, CreateMessage, EditMessage, Http, MessageId,
};

use crate::discord::components::claim_button;
use crate::discord::embeds::ace_embed;
use crate::util::{channel, str_field};

pub(crate) async fn post_ace_request(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let request_id = str_field(p, "request_id").ok_or("missing request_id")?;
    let message = channel
        .send_message(http, build_request_message(p, request_id))
        .await
        .map_err(|e| format!("send_message failed: {e}"))?;
    Ok(Some(json!({ "message_id": message.id.get().to_string() })))
}

/// The channel post for a new ACE request. Split out from [`post_ace_request`] so its payload can be
/// asserted without a live Discord connection.
///
/// The embed carries caller text (`details`, `event_title`). Discord does not parse mentions in an
/// embed today, but an explicit empty `allowed_mentions` keeps that text inert if any of it moves
/// into `content` (#933).
fn build_request_message(p: &Value, request_id: &str) -> CreateMessage {
    CreateMessage::new()
        .embed(ace_embed(p, 0, false))
        .components(vec![CreateActionRow::Buttons(vec![claim_button(
            request_id,
        )])])
        .allowed_mentions(CreateAllowedMentions::new())
}

pub(crate) async fn notify_ace_claim(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let message_id: u64 = str_field(p, "message_id")
        .ok_or("missing message_id")?
        .parse()
        .map_err(|_| "message_id not a snowflake".to_string())?;
    let claims_count = p.get("claims_count").and_then(Value::as_i64).unwrap_or(0);
    let filled = p.get("filled").and_then(Value::as_bool).unwrap_or(false);
    let request_id = str_field(p, "request_id").ok_or("missing request_id")?;

    channel
        .edit_message(
            http,
            MessageId::new(message_id),
            build_claim_edit(p, claims_count, filled, request_id),
        )
        .await
        .map_err(|e| format!("edit_message failed: {e}"))?;
    Ok(None)
}

/// The edit that updates a posted request's claim count. Its embed adds claimer names and notes to
/// the caller text [`build_request_message`] carries, so it sets the same empty `allowed_mentions`.
fn build_claim_edit(p: &Value, claims_count: i64, filled: bool, request_id: &str) -> EditMessage {
    // Keep the Claim button while slots remain; drop it once full.
    let components = if filled {
        vec![]
    } else {
        vec![CreateActionRow::Buttons(vec![claim_button(request_id)])]
    };
    EditMessage::new()
        .embed(ace_embed(p, claims_count, filled))
        .components(components)
        .allowed_mentions(CreateAllowedMentions::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request whose caller text carries every mention form Discord would parse in `content`.
    fn payload_with_mentions() -> Value {
        json!({
            "channel_id": "1",
            "request_id": "req-1",
            "event_title": "Fly-In @everyone",
            "details": "@everyone <@&123> <@456>",
            "slots": 2,
            "claimers": [{ "name": "A Controller", "notes": "@here <@789>" }],
        })
    }

    /// `allowed_mentions` is present with an empty `parse` and no user or role ids, which is what
    /// makes every mention in the message inert.
    fn assert_mentions_inert(json: &Value) {
        let mentions = json
            .get("allowed_mentions")
            .expect("allowed_mentions must be set, or Discord parses by default");
        let parse = mentions.get("parse").and_then(Value::as_array);
        assert_eq!(parse.map(Vec::len), Some(0), "got {mentions:?}");
        for key in ["users", "roles"] {
            let listed = mentions
                .get(key)
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            assert_eq!(listed, 0, "{key} listed in {mentions:?}");
        }
    }

    fn embeds_text(json: &Value) -> String {
        json.get("embeds").map(Value::to_string).unwrap_or_default()
    }

    #[test]
    fn an_at_everyone_in_an_ace_request_cannot_ping_the_channel() {
        let json = serde_json::to_value(build_request_message(&payload_with_mentions(), "req-1"))
            .expect("serialize");

        assert_mentions_inert(&json);
        // The message really carries the caller's mentions, or the assertion above is vacuous.
        let embeds = embeds_text(&json);
        assert!(
            embeds.contains("@everyone <@&123> <@456>"),
            "details missing: {embeds}"
        );
        assert!(
            embeds.contains("Fly-In @everyone"),
            "event_title missing: {embeds}"
        );
    }

    #[test]
    fn an_at_everyone_in_an_open_or_filled_claim_edit_cannot_ping_the_channel() {
        for (claims_count, filled) in [(1, false), (2, true)] {
            let edit = build_claim_edit(&payload_with_mentions(), claims_count, filled, "req-1");
            let json = serde_json::to_value(edit).expect("serialize");

            assert_mentions_inert(&json);
            let embeds = embeds_text(&json);
            assert!(
                embeds.contains("@everyone <@&123> <@456>"),
                "filled={filled}: {embeds}"
            );
            assert!(
                embeds.contains("@here <@789>"),
                "filled={filled}: claimer notes missing"
            );
        }
    }

    #[test]
    fn the_request_message_carries_the_claim_button_for_its_request() {
        let json = serde_json::to_value(build_request_message(&payload_with_mentions(), "req-1"))
            .expect("serialize");

        let buttons = &json["components"][0]["components"];
        assert_eq!(buttons.as_array().map(Vec::len), Some(1), "got {buttons}");
        assert_eq!(buttons[0]["custom_id"], "ace_claim:req-1");
    }

    #[test]
    fn an_open_claim_edit_keeps_the_claim_button() {
        let json = serde_json::to_value(build_claim_edit(
            &payload_with_mentions(),
            1,
            false,
            "req-1",
        ))
        .expect("serialize");

        let buttons = &json["components"][0]["components"];
        assert_eq!(buttons.as_array().map(Vec::len), Some(1), "got {buttons}");
        assert_eq!(buttons[0]["custom_id"], "ace_claim:req-1");
    }

    /// An empty `components` array clears the button; a missing field would leave the old one.
    #[test]
    fn a_filled_claim_edit_drops_the_claim_button() {
        let json =
            serde_json::to_value(build_claim_edit(&payload_with_mentions(), 2, true, "req-1"))
                .expect("serialize");

        assert_eq!(json["components"], json!([]));
    }
}
