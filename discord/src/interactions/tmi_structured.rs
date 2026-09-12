use ois_client::DiscordTmiInfo;
use serenity::all::{ComponentInteraction, Context};

use crate::util::ephemeral;

/// The ephemeral reply body for a "View structured" click: the plain-English breakdown when the
/// TMI was entered via the structured form, else a note that none exists (a raw-typed TMI).
fn structured_reply(info: &DiscordTmiInfo) -> String {
    match &info.decoded {
        Some(decoded) => decoded.clone(),
        None => "This TMI was entered as free-form text — no structured breakdown available."
            .to_string(),
    }
}

/// "View structured" button on a TMI post → look up its structured breakdown and reply ephemerally.
/// custom_id = tmiV:{tmi_id}
pub(crate) async fn handle(
    ctx: &Context,
    mc: &ComponentInteraction,
    api: &ois_client::OisClient,
    tmi_id: &str,
) {
    let content = match api.tmi_info(tmi_id).await {
        Ok(info) => structured_reply(&info),
        Err(e) => tmi_lookup_error(&e, tmi_id),
    };
    if let Err(e) = mc.create_response(&ctx.http, ephemeral(&content)).await {
        tracing::error!(error = %e, tmi = tmi_id, "failed to send structured-view reply");
    }
}

/// Map a lookup client-error to a user-facing message. A 404 specifically means the TMI row was
/// hard-deleted by `prune_history` after expiring — an old post's button can outlive the row it
/// points at, so that case gets a message that won't send the user into a "try again" dead end.
fn tmi_lookup_error(e: &ois_client::ClientError, tmi_id: &str) -> String {
    match e.status() {
        Some(404) => {
            "This TMI is no longer available (it may have been cleaned up after expiring)."
                .to_string()
        }
        _ => {
            tracing::error!(error = %e, tmi = tmi_id, "tmi info lookup failed");
            "Couldn't look up that TMI right now — please try again.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_reply_uses_decoded_when_present() {
        let info = DiscordTmiInfo {
            restriction: "raw line".to_string(),
            decoded: Some("2 in trail via CAMRN".to_string()),
        };
        assert_eq!(structured_reply(&info), "2 in trail via CAMRN");
    }

    #[test]
    fn structured_reply_falls_back_for_a_raw_typed_tmi() {
        let info = DiscordTmiInfo {
            restriction: "raw line".to_string(),
            decoded: None,
        };
        assert_eq!(
            structured_reply(&info),
            "This TMI was entered as free-form text — no structured breakdown available."
        );
    }

    /// A 404 (the TMI row was hard-deleted by retention pruning) gets a message that doesn't send
    /// the user into a "try again" dead end, unlike every other error.
    #[test]
    fn tmi_lookup_error_distinguishes_a_pruned_tmi_from_a_generic_failure() {
        assert_eq!(
            tmi_lookup_error(&ois_client::ClientError::Status(404), "tmi-1"),
            "This TMI is no longer available (it may have been cleaned up after expiring)."
        );
        assert_eq!(
            tmi_lookup_error(&ois_client::ClientError::Status(500), "tmi-1"),
            "Couldn't look up that TMI right now — please try again."
        );
    }
}
