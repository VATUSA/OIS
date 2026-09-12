use serde_json::Value;
use serenity::all::{Colour, CreateEmbed};

use crate::util::{hhmm, str_field};

/// The ACE request embed: details + an "X/N claimed" slot meter + the claimer lines. Shared by the
/// initial post (0/N) and the claim/release notify (edits the same message).
pub(crate) fn ace_embed(p: &Value, claims_count: i64, filled: bool) -> CreateEmbed {
    let slots = p.get("slots").and_then(Value::as_i64).unwrap_or(1);
    let mut embed = CreateEmbed::new()
        .title("ACE coverage request")
        .colour(Colour::new(if filled { 0x57F287 } else { 0x5865F2 }));
    if let Some(event) = str_field(p, "event_title") {
        embed = embed.field("Event", event, false);
    }
    if let Some(artcc) = str_field(p, "artcc_id") {
        embed = embed.field("ARTCC", artcc, true);
    }
    if let Some(position) = str_field(p, "position") {
        embed = embed.field("Position", position, true);
    }
    embed = embed.field("Slots", format!("{claims_count}/{slots} claimed"), true);
    if let Some(details) = str_field(p, "details") {
        embed = embed.description(details);
    }
    if let Some(claimers) = p.get("claimers").and_then(Value::as_array)
        && !claimers.is_empty()
    {
        let lines: Vec<String> = claimers
            .iter()
            .map(|c| {
                let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
                let window = match (
                    c.get("start_time").and_then(Value::as_str),
                    c.get("end_time").and_then(Value::as_str),
                ) {
                    (Some(s), Some(e)) => format!(" · {}–{}z", hhmm(s), hhmm(e)),
                    _ => String::new(),
                };
                let notes = c
                    .get("notes")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty())
                    .map(|n| format!(" — {n}"))
                    .unwrap_or_default();
                format!("• **{name}**{window}{notes}")
            })
            .collect();
        embed = embed.field("Claimed by", lines.join("\n"), false);
    }
    embed
}
