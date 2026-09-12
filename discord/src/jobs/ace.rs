use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{CreateActionRow, CreateMessage, EditMessage, Http, MessageId};

use crate::discord::components::claim_button;
use crate::discord::embeds::ace_embed;
use crate::util::{channel, str_field};

pub(crate) async fn post_ace_request(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let request_id = str_field(p, "request_id").ok_or("missing request_id")?;
    let button = claim_button(request_id);
    let message = channel
        .send_message(
            http,
            CreateMessage::new()
                .embed(ace_embed(p, 0, false))
                .components(vec![CreateActionRow::Buttons(vec![button])]),
        )
        .await
        .map_err(|e| format!("send_message failed: {e}"))?;
    Ok(Some(json!({ "message_id": message.id.get().to_string() })))
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

    // Keep the Claim button while slots remain; drop it once full.
    let components = if filled {
        vec![]
    } else {
        vec![CreateActionRow::Buttons(vec![claim_button(request_id)])]
    };
    channel
        .edit_message(
            http,
            MessageId::new(message_id),
            EditMessage::new()
                .embed(ace_embed(p, claims_count, filled))
                .components(components),
        )
        .await
        .map_err(|e| format!("edit_message failed: {e}"))?;
    Ok(None)
}
