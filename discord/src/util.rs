use serde_json::Value;
use serenity::all::{ChannelId, CreateInteractionResponse, CreateInteractionResponseMessage};

/// Custom-id prefix for the "Claim" button on an ACE request embed; shared by the button builder
/// (`discord::components::claim_button`) and the interaction handler that strips it
/// (`interactions::ace_claim`).
pub(crate) const ACE_CLAIM_PREFIX: &str = "ace_claim:";

/// Custom-id prefix for the "View structured" button on a TMI post; shared by the button builder
/// (`jobs::tmi`) and the interaction handler that strips it (`interactions::tmi_structured`).
pub(crate) const TMI_STRUCTURED_PREFIX: &str = "tmiV:";

/// Resolve the `channel_id` snowflake from a job payload.
pub(crate) fn channel(p: &Value) -> Result<ChannelId, String> {
    let id: u64 = str_field(p, "channel_id")
        .ok_or("missing channel_id")?
        .parse()
        .map_err(|_| "channel_id not a snowflake".to_string())?;
    Ok(ChannelId::new(id))
}

pub(crate) fn str_field<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Clamp to `max` characters (Discord thread names cap at 100), on a char boundary.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Extract "HHMM" from an ISO-8601 timestamp (e.g. `2026-08-23T23:30:00+00:00` → `2330`).
pub(crate) fn hhmm(iso: &str) -> String {
    iso.split('T')
        .nth(1)
        .map(|t| t.chars().take(5).filter(char::is_ascii_digit).collect())
        .unwrap_or_default()
}

/// An ephemeral text-only response.
pub(crate) fn ephemeral(content: &str) -> CreateInteractionResponse {
    CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(content)
            .ephemeral(true),
    )
}
