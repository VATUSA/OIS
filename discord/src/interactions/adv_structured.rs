use ois_client::DiscordAdvisoryInfo;
use serenity::all::{ComponentInteraction, Context};

use crate::util::ephemeral;

/// The ephemeral reply body for a "View structured" click on an advisory.
///
/// Advisories differ from TMIs here: a TMI has a `decoded` plain-English rendering, but an advisory's
/// document *is* its rendering — it is already in the channel above the button. What the structured
/// form adds is the field breakdown, so that is what this shows, pretty-printed.
///
/// A raw-typed advisory has no breakdown and says so, rather than showing an empty object.
fn structured_reply(info: &DiscordAdvisoryInfo) -> String {
    match &info.structured {
        Some(fields) => {
            let pretty = serde_json::to_string_pretty(fields)
                .unwrap_or_else(|_| "(breakdown could not be rendered)".to_string());
            format!("**{}**\n```json\n{}\n```", info.kind, truncated(&pretty))
        }
        None => "This advisory was entered as free-form text — no structured breakdown available."
            .to_string(),
    }
}

/// Discord's message limit applies to an ephemeral reply too, and a structured breakdown is
/// unbounded — a multi-segment reroute's route table can be long. Clipping a *diagnostic* view is
/// acceptable where clipping the posted document is not, so unlike `jobs::advisory` this truncates
/// rather than splits: the document itself is already in the channel intact.
fn truncated(pretty: &str) -> String {
    const MAX: usize = 1800; // leaves room for the kind header and the fence
    if pretty.chars().count() <= MAX {
        return pretty.to_string();
    }
    let clipped: String = pretty.chars().take(MAX).collect();
    format!("{clipped}\n… (truncated; the full document is posted above)")
}

/// "View structured" button on an advisory post → look up its breakdown and reply ephemerally.
/// custom_id = advV:{advisory_id}
pub(crate) async fn handle(
    ctx: &Context,
    mc: &ComponentInteraction,
    api: &ois_client::OisClient,
    advisory_id: &str,
) {
    let content = match api.advisory_info(advisory_id).await {
        Ok(info) => structured_reply(&info),
        Err(e) => lookup_error(&e, advisory_id),
    };
    if let Err(e) = mc.create_response(&ctx.http, ephemeral(&content)).await {
        tracing::error!(error = %e, advisory = advisory_id, "failed to send structured-view reply");
    }
}

/// Map a lookup client-error to a user-facing message.
///
/// A 404 here means something different from the TMI case: advisories are never hard-deleted once
/// published (`repos::tmu::delete_advisory` only touches drafts), so a missing row means the button
/// belongs to a draft that was abandoned before publishing.
fn lookup_error(e: &ois_client::ClientError, advisory_id: &str) -> String {
    match e.status() {
        Some(404) => "This advisory no longer exists (an abandoned draft).".to_string(),
        _ => {
            tracing::error!(error = %e, advisory = advisory_id, "advisory info lookup failed");
            "Couldn't look up that advisory right now — please try again.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(structured: Option<serde_json::Value>) -> DiscordAdvisoryInfo {
        DiscordAdvisoryInfo {
            kind: "reroute".to_string(),
            structured,
        }
    }

    #[test]
    fn a_structured_advisory_shows_its_breakdown_under_its_kind() {
        let reply = structured_reply(&info(Some(serde_json::json!({"name": "NO_J75_3_PARTIAL"}))));
        assert!(reply.contains("**reroute**"), "{reply}");
        assert!(reply.contains("NO_J75_3_PARTIAL"), "{reply}");
    }

    /// AC 5: a raw-typed advisory must report cleanly, not show an empty breakdown.
    #[test]
    fn a_raw_typed_advisory_says_it_has_no_breakdown() {
        let reply = structured_reply(&info(None));
        assert!(reply.contains("free-form text"), "{reply}");
        assert!(!reply.contains("```"), "no empty code block: {reply}");
    }

    /// An ephemeral reply is still subject to the message limit, and a breakdown is unbounded.
    #[test]
    fn an_oversized_breakdown_is_clipped_and_says_so() {
        let long = serde_json::json!({ "route": "X".repeat(4000) });
        let reply = structured_reply(&info(Some(long)));
        assert!(
            reply.chars().count() < 2000,
            "len {}",
            reply.chars().count()
        );
        assert!(reply.contains("truncated"), "{reply}");
    }

    /// A 404 means an abandoned draft here, not a pruned row — published advisories are never
    /// hard-deleted, so the message must not send the reader looking for something that expired.
    #[test]
    fn a_missing_advisory_is_reported_as_an_abandoned_draft() {
        let msg = lookup_error(&ois_client::ClientError::Status(404), "adv-1");
        assert!(msg.contains("abandoned draft"), "{msg}");

        let other = lookup_error(&ois_client::ClientError::Status(500), "adv-1");
        assert!(other.contains("try again"), "{other}");
    }
}
