use serenity::all::{ComponentInteraction, Context};

use crate::util::ephemeral;

/// Human label for an availability status (matches the button legend).
fn availability_label(status: &str) -> &'static str {
    match status {
        "available" => "🟢 Available",
        "partial" => "🟡 Partially available",
        "unavailable" => "🔴 Unavailable",
        _ => "recorded",
    }
}

/// Event-thread availability buttons (🟢🟡🔴): record the presser's availability for the event.
/// custom_id = evtavail:{green|yellow|red}:{event_id}
pub(crate) async fn handle(
    ctx: &Context,
    mc: &ComponentInteraction,
    api: &ois_client::OisClient,
    rest: &str,
) {
    let mut parts = rest.splitn(2, ':');
    let color = parts.next().unwrap_or("");
    let event_id = parts.next().unwrap_or("");
    let status = match color {
        "green" => "available",
        "yellow" => "partial",
        "red" => "unavailable",
        _ => "",
    };
    let content = if status.is_empty() || event_id.is_empty() {
        "Unknown availability option.".to_string()
    } else {
        let discord_user = mc.user.id.get().to_string();
        match api.set_availability(event_id, &discord_user, status).await {
            Ok(r) if r.ok => format!(
                "✅ Recorded your availability: {}",
                availability_label(status)
            ),
            Ok(r) => match r.reason.as_deref() {
                Some("unlinked") => {
                    "Your Discord isn't linked to OIS yet — link it via VATUSA to respond."
                        .to_string()
                }
                Some("forbidden") => {
                    "Only NTMOs (or authorized DCC staff) can indicate availability here."
                        .to_string()
                }
                _ => "Couldn't record that response.".to_string(),
            },
            Err(_) => {
                "Something went wrong recording your availability. Try again shortly.".to_string()
            }
        }
    };
    let _ = mc.create_response(&ctx.http, ephemeral(&content)).await;
}
