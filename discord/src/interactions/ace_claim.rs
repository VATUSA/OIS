use ois_client::{ClientError, OisClient};
use serenity::all::{
    ActionRowComponent, ComponentInteraction, ComponentInteractionDataKind, Context,
    CreateActionRow, CreateInputText, CreateInteractionResponse, CreateInteractionResponseMessage,
    CreateModal, InputTextStyle, ModalInteraction,
};

use crate::discord::components::claim_components;
use crate::util::ephemeral;

const ACE_NOTES_PREFIX: &str = "aceN:";

/// Map a claim client-error to a user-facing message.
fn claim_error(e: &ClientError) -> String {
    match e.status() {
        Some(403) => "No OIS account is linked to your Discord. Add your Discord to your VATUSA \
                      profile, then sign in to OIS to sync it."
            .to_string(),
        Some(409) => "This request is full, or you've already claimed a slot.".to_string(),
        Some(404) => "That request no longer exists.".to_string(),
        Some(400) => "That window isn't valid for the event — try again.".to_string(),
        _ => {
            tracing::error!(error = %e, "ace claim/info call failed");
            "Couldn't do that right now — please try again.".to_string()
        }
    }
}

/// Notes modal submit → claim with the times carried in the modal id + the typed notes. No-op if
/// the modal isn't one of ours (custom_id doesn't carry the ACE-notes prefix).
pub(crate) async fn handle_modal_submit(ctx: &Context, ms: &ModalInteraction, api: &OisClient) {
    let Some(rest) = ms.data.custom_id.strip_prefix(ACE_NOTES_PREFIX) else {
        return;
    };
    let parts: Vec<&str> = rest.splitn(3, ':').collect();
    if parts.len() != 3 {
        return;
    }
    let (request_id, start, end) = (parts[0], parts[1], parts[2]);
    let mut notes = None;
    for row in &ms.data.components {
        for comp in &row.components {
            if let ActionRowComponent::InputText(it) = comp
                && it.custom_id == "notes"
            {
                notes = it
                    .value
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
            }
        }
    }
    let discord_user = ms.user.id.get().to_string();
    let content = match api
        .claim_ace_via_discord(
            request_id,
            &discord_user,
            notes.as_deref(),
            Some(start),
            Some(end),
        )
        .await
    {
        Ok(_) => format!("✅ Claimed {start}–{end}z — thanks for covering this."),
        Err(e) => claim_error(&e),
    };
    // The modal came from the ephemeral picker → collapse it into the result.
    let resp = CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new()
            .content(content)
            .components(vec![]),
    );
    if let Err(e) = ms.create_response(&ctx.http, resp).await {
        tracing::error!(error = %e, "failed to finalize claim from modal");
    }
}

/// Original "Claim" button on the request embed → open the ephemeral time-picker.
pub(crate) async fn handle_claim_button(
    ctx: &Context,
    mc: &ComponentInteraction,
    api: &OisClient,
    request_id: &str,
) {
    let info = match api.ace_info(request_id).await {
        Ok(i) => i,
        Err(e) => {
            let _ = mc
                .create_response(&ctx.http, ephemeral(&claim_error(&e)))
                .await;
            return;
        }
    };
    let content = format!(
        "**{}** · {} — {}/{} claimed. Pick when you can start and end, then Claim:",
        info.event_title, info.window_label, info.claims_count, info.slots
    );
    let resp = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(content)
            .components(claim_components(request_id, &info.time_options, "-", "-"))
            .ephemeral(true),
    );
    if let Err(e) = mc.create_response(&ctx.http, resp).await {
        tracing::error!(error = %e, "failed to open claim picker");
    }
}

/// Start/End dropdown → carry the new selection in the re-rendered custom_ids. `cid` is the raw
/// custom_id, already known to start with "aceS:" or "aceE:".
pub(crate) async fn handle_select(
    ctx: &Context,
    mc: &ComponentInteraction,
    api: &OisClient,
    cid: &str,
) {
    let parts: Vec<&str> = cid.splitn(4, ':').collect();
    let (tag, request_id, mut start, mut end) = (
        parts[0],
        parts[1].to_string(),
        parts[2].to_string(),
        parts[3].to_string(),
    );
    if let ComponentInteractionDataKind::StringSelect { values } = &mc.data.kind
        && let Some(v) = values.first()
    {
        if tag == "aceS" {
            start = v.clone();
        } else {
            end = v.clone();
        }
    }
    let Ok(info) = api.ace_info(&request_id).await else {
        return;
    };
    let resp = CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new().components(claim_components(
            &request_id,
            &info.time_options,
            &start,
            &end,
        )),
    );
    if let Err(e) = mc.create_response(&ctx.http, resp).await {
        tracing::error!(error = %e, "failed to update claim picker");
    }
}

/// "Claim slot" confirm button → pop a modal for optional notes; the submit does the claim. `rest`
/// is the custom_id with the `aceG:` prefix already stripped.
pub(crate) async fn handle_confirm(ctx: &Context, mc: &ComponentInteraction, rest: &str) {
    let parts: Vec<&str> = rest.splitn(3, ':').collect();
    let (request_id, start, end) = (parts[0], parts[1], parts[2]);
    if start == "-" || end == "-" {
        let _ = mc
            .create_response(
                &ctx.http,
                ephemeral("Pick both a start and end time first."),
            )
            .await;
        return;
    }
    let modal = CreateModal::new(
        format!("{ACE_NOTES_PREFIX}{request_id}:{start}:{end}"),
        format!("Claim {start}–{end}z"),
    )
    .components(vec![CreateActionRow::InputText(
        CreateInputText::new(InputTextStyle::Paragraph, "Notes (optional)", "notes")
            .required(false),
    )]);
    if let Err(e) = mc
        .create_response(&ctx.http, CreateInteractionResponse::Modal(modal))
        .await
    {
        tracing::error!(error = %e, "failed to open notes modal");
    }
}
