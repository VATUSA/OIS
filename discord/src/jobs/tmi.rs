use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{
    ButtonStyle, CreateActionRow, CreateAllowedMentions, CreateButton, CreateMessage, Http,
};

use crate::util::{TMI_STRUCTURED_PREFIX, channel, str_field};

/// The requesting/providing header line, when at least one facility is known — matching the old
/// embed's behaviour of showing each field independently rather than requiring both (a facility can
/// be blank on a TMI mid-edit via `update_tmi`, which — unlike `create_tmi` — doesn't reject an empty
/// `requesting`/`providing`).
fn facility_header(requesting: Option<&str>, providing: Option<&str>) -> Option<String> {
    match (requesting, providing) {
        (Some(req), Some(prov)) => Some(format!("**{req} → {prov}**")),
        (Some(req), None) => Some(format!("**{req}**")),
        (None, Some(prov)) => Some(format!("**{prov}**")),
        (None, None) => None,
    }
}

pub(crate) async fn post_tmi(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let restriction = str_field(p, "restriction").unwrap_or("(no restriction text)");
    // `restriction`/`encode()` never carries the requesting/providing facilities — those live on
    // the TMI row, not the NTML line (see backend/src/tmi.rs's own doc comment) — so a plain post
    // of just the raw line would silently drop them versus the old embed's dedicated fields.
    let content = match facility_header(str_field(p, "requesting"), str_field(p, "providing")) {
        Some(header) => format!("{header}\n{restriction}"),
        None => restriction.to_string(),
    };

    let mut message = CreateMessage::new()
        .content(content)
        // The restriction is free-form text a TMU controller typed (`create_tmi`/`update_tmi` apply
        // no mention-syntax filtering); unlike the embed this replaces — which Discord never parses
        // for mentions — plain message content DOES get parsed by default when this is omitted,
        // turning an accidental "@everyone" in a TMI into a real mass-ping of the channel.
        .allowed_mentions(CreateAllowedMentions::new());
    if let Some(tmi_id) = str_field(p, "tmi_id") {
        let button = CreateButton::new(format!("{TMI_STRUCTURED_PREFIX}{tmi_id}"))
            .label("View structured")
            .style(ButtonStyle::Secondary);
        message = message.components(vec![CreateActionRow::Buttons(vec![button])]);
    }

    let message = channel
        .send_message(http, message)
        .await
        .map_err(|e| format!("send_message failed: {e}"))?;
    Ok(Some(json!({ "message_id": message.id.get().to_string() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facility_header_shows_both_when_present() {
        assert_eq!(
            facility_header(Some("ZDC"), Some("ZNY")),
            Some("**ZDC → ZNY**".to_string())
        );
    }

    /// Matches the old embed's behaviour of showing each facility field independently — a TMI
    /// mid-edit via `update_tmi` can have one blanked without the other, and the header should still
    /// surface whichever facility is known rather than dropping both.
    #[test]
    fn facility_header_shows_whichever_one_is_present() {
        assert_eq!(
            facility_header(Some("ZDC"), None),
            Some("**ZDC**".to_string())
        );
        assert_eq!(
            facility_header(None, Some("ZNY")),
            Some("**ZNY**".to_string())
        );
    }

    #[test]
    fn facility_header_is_none_when_neither_is_present() {
        assert_eq!(facility_header(None, None), None);
    }
}
