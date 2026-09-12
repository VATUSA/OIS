mod ace_claim;
mod event_availability;

use ois_client::OisClient;
use serenity::all::{Context, EventHandler, Interaction, Ready};

use crate::snapshot::snapshot_and_push;
use crate::util::ACE_CLAIM_PREFIX;

pub(crate) struct Handler {
    pub(crate) api: OisClient,
}

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!(bot = %ready.user.name, guilds = ready.guilds.len(), "discord bot connected");
        // Push the guild snapshot so the admin config can offer channel/role dropdowns.
        let api = self.api.clone();
        let http = ctx.http.clone();
        tokio::spawn(async move {
            if let Err(e) = snapshot_and_push(&http, &api).await {
                tracing::warn!(reason = %e, "initial guild snapshot failed");
            }
        });
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // Notes modal submit → claim with the times carried in the modal id + the typed notes.
        if let Interaction::Modal(ms) = &interaction {
            ace_claim::handle_modal_submit(&ctx, ms, &self.api).await;
            return;
        }

        let Interaction::Component(mc) = interaction else {
            return;
        };
        let cid = mc.data.custom_id.clone();

        // Event-thread availability buttons (🟢🟡🔴): record the presser's availability for the event.
        // custom_id = evtavail:{green|yellow|red}:{event_id}
        if let Some(rest) = cid.strip_prefix("evtavail:") {
            event_availability::handle(&ctx, &mc, &self.api, rest).await;
            return;
        }

        // 1) Original "Claim" button on the request embed → open the ephemeral time-picker.
        if let Some(request_id) = cid.strip_prefix(ACE_CLAIM_PREFIX) {
            ace_claim::handle_claim_button(&ctx, &mc, &self.api, request_id).await;
            return;
        }

        // 2) Start/End dropdown → carry the new selection in the re-rendered custom_ids.
        if cid.starts_with("aceS:") || cid.starts_with("aceE:") {
            ace_claim::handle_select(&ctx, &mc, &self.api, &cid).await;
            return;
        }

        // 3) "Claim slot" confirm button → pop a modal for optional notes; the submit does the claim.
        if let Some(rest) = cid.strip_prefix("aceG:") {
            ace_claim::handle_confirm(&ctx, &mc, rest).await;
        }
    }
}
