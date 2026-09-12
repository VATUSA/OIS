use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{Colour, CreateEmbed, CreateMessage, Http};

use crate::util::{channel, str_field};

pub(crate) async fn post_tmi(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let mut embed = CreateEmbed::new()
        .title("Traffic management initiative")
        .colour(Colour::new(0xFEE75C));
    if let Some(requesting) = str_field(p, "requesting") {
        embed = embed.field("Requesting", requesting, true);
    }
    if let Some(providing) = str_field(p, "providing") {
        embed = embed.field("Providing", providing, true);
    }
    if let Some(restriction) = str_field(p, "restriction") {
        embed = embed.field("Restriction", restriction, false);
    }
    let message = channel
        .send_message(http, CreateMessage::new().embed(embed))
        .await
        .map_err(|e| format!("send_message failed: {e}"))?;
    Ok(Some(json!({ "message_id": message.id.get().to_string() })))
}
