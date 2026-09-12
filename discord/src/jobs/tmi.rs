use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{ButtonStyle, CreateActionRow, CreateButton, CreateMessage, Http};

use crate::util::{TMI_STRUCTURED_PREFIX, channel, str_field};

pub(crate) async fn post_tmi(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let restriction = str_field(p, "restriction").unwrap_or("(no restriction text)");
    // `restriction`/`encode()` never carries the requesting/providing facilities — those live on
    // the TMI row, not the NTML line (see backend/src/tmi.rs's own doc comment) — so a plain post
    // of just the raw line would silently drop them versus the old embed's dedicated fields.
    let content = match (str_field(p, "requesting"), str_field(p, "providing")) {
        (Some(req), Some(prov)) => format!("**{req} → {prov}**\n{restriction}"),
        _ => restriction.to_string(),
    };

    let mut message = CreateMessage::new().content(content);
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
