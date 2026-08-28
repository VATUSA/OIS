//! OIS Discord bot.
//!
//! Two responsibilities, both mediated by the backend (the bot owns no data):
//!   1. **Drain the outbound-job queue** — poll `integration.outbound_jobs` via the API, perform the
//!      Discord side-effect (post/edit an embed), and ack the job with any ids it produced.
//!   2. **Handle interactions** — when a user clicks "Claim" on an ACE request, call back into the API
//!      as a service account; the backend resolves the Discord user to the linked OIS account.
//!
//! Config (env / `.env`): `DISCORD_BOT_TOKEN`, `OIS_API_BASE`, `OIS_API_TOKEN` (an `ois_sa_…`
//! service-account token holding the BOT role), optional `OIS_POLL_SECS` (default 5).

use std::sync::Arc;
use std::time::Duration;

use ois_client::{GuildChannelSnap, GuildRoleSnap, GuildSnap, OisClient, OutboundJob};
use serde_json::{Value, json};
use serenity::all::{
    ActionRowComponent, ButtonStyle, ChannelId, ChannelType, Colour, ComponentInteractionDataKind,
    Context, CreateActionRow, CreateAllowedMentions, CreateButton, CreateEmbed, CreateInputText,
    CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, CreateModal,
    CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption, CreateThread, EditMessage,
    EventHandler, GatewayIntents, Http, InputTextStyle, Interaction, MessageId, Ready, RoleId,
    UserId,
};

const ACE_CLAIM_PREFIX: &str = "ace_claim:";
const ACE_NOTES_PREFIX: &str = "aceN:";

struct Config {
    discord_token: String,
    api_base: String,
    api_token: String,
    poll: Duration,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let get = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("missing required env var {k}"))
        };
        let poll = std::env::var("OIS_POLL_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(5);
        Ok(Self {
            discord_token: get("DISCORD_BOT_TOKEN")?,
            api_base: get("OIS_API_BASE")?,
            api_token: get("OIS_API_TOKEN")?,
            poll: Duration::from_secs(poll),
        })
    }
}

struct Handler {
    api: OisClient,
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
            let content = match self
                .api
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
            return;
        }

        let Interaction::Component(mc) = interaction else {
            return;
        };
        let cid = mc.data.custom_id.clone();

        // Event-thread availability buttons (🟢🟡🔴): record the presser's availability for the event.
        // custom_id = evtavail:{green|yellow|red}:{event_id}
        if let Some(rest) = cid.strip_prefix("evtavail:") {
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
                match self
                    .api
                    .set_availability(event_id, &discord_user, status)
                    .await
                {
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
                        "Something went wrong recording your availability. Try again shortly."
                            .to_string()
                    }
                }
            };
            let _ = mc.create_response(&ctx.http, ephemeral(&content)).await;
            return;
        }

        // 1) Original "Claim" button on the request embed → open the ephemeral time-picker.
        if let Some(request_id) = cid.strip_prefix(ACE_CLAIM_PREFIX) {
            let info = match self.api.ace_info(request_id).await {
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
            return;
        }

        // 2) Start/End dropdown → carry the new selection in the re-rendered custom_ids.
        if cid.starts_with("aceS:") || cid.starts_with("aceE:") {
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
            let Ok(info) = self.api.ace_info(&request_id).await else {
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
            return;
        }

        // 3) "Claim slot" confirm button → pop a modal for optional notes; the submit does the claim.
        if let Some(rest) = cid.strip_prefix("aceG:") {
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
    }
}

/// Human label for an availability status (matches the button legend).
fn availability_label(status: &str) -> &'static str {
    match status {
        "available" => "🟢 Available",
        "partial" => "🟡 Partially available",
        "unavailable" => "🔴 Unavailable",
        _ => "recorded",
    }
}

/// An ephemeral text-only response.
fn ephemeral(content: &str) -> CreateInteractionResponse {
    CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(content)
            .ephemeral(true),
    )
}

/// Map a claim client-error to a user-facing message.
fn claim_error(e: &ois_client::ClientError) -> String {
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

/// The claim picker: Start + End dropdowns (Zulu HHMM) and a Claim button. All three custom_ids carry
/// the current `(start, end)` so any change re-renders with the state intact; the button enables once
/// both are chosen.
fn claim_components(
    request_id: &str,
    options: &[String],
    start: &str,
    end: &str,
) -> Vec<CreateActionRow> {
    let menu = |tag: char, placeholder: &str, chosen: &str| {
        let opts: Vec<CreateSelectMenuOption> = options
            .iter()
            .map(|o| {
                CreateSelectMenuOption::new(format!("{o}z"), o.clone())
                    .default_selection(o == chosen)
            })
            .collect();
        CreateActionRow::SelectMenu(
            CreateSelectMenu::new(
                format!("ace{tag}:{request_id}:{start}:{end}"),
                CreateSelectMenuKind::String { options: opts },
            )
            .placeholder(placeholder),
        )
    };
    let ready = start != "-" && end != "-";
    let confirm = CreateButton::new(format!("aceG:{request_id}:{start}:{end}"))
        .label("Claim slot")
        .style(ButtonStyle::Success)
        .disabled(!ready);
    vec![
        menu('S', "Start time (Zulu)", start),
        menu('E', "End time (Zulu)", end),
        CreateActionRow::Buttons(vec![confirm]),
    ]
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let _ = dotenvy::dotenv();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("{e}");
            eprintln!("ois-discord: {e}. See the header of src/main.rs for required config.");
            std::process::exit(1);
        }
    };

    let api = OisClient::new(config.api_base.clone(), config.api_token.clone());

    // No privileged intents needed — component interactions arrive regardless.
    let mut client = match serenity::Client::builder(&config.discord_token, GatewayIntents::empty())
        .event_handler(Handler { api: api.clone() })
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to build discord client");
            std::process::exit(1);
        }
    };

    // The job loop performs Discord actions via its own Http handle, independent of the gateway.
    let http = client.http.clone();
    tokio::spawn(job_loop(api, http, config.poll));

    if let Err(e) = client.start().await {
        tracing::error!(error = %e, "discord gateway error");
        std::process::exit(1);
    }
}

/// Poll → perform → ack, forever. One job's failure never stops the loop.
async fn job_loop(api: OisClient, http: Arc<Http>, poll: Duration) {
    loop {
        match api.lease_jobs(10).await {
            Ok(jobs) => {
                for job in jobs {
                    let id = job.id.clone();
                    match perform_job(&api, &http, &job).await {
                        Ok(result) => {
                            if let Err(e) = api.ack_job(&id, true, result, None).await {
                                tracing::error!(error = %e, job = %id, "ack(success) failed");
                            }
                        }
                        Err(reason) => {
                            tracing::warn!(job = %id, reason, "job failed; nacking for retry");
                            if let Err(e) = api.ack_job(&id, false, None, Some(&reason)).await {
                                tracing::error!(error = %e, job = %id, "ack(failure) failed");
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::error!(error = %e, "lease failed"),
        }
        tokio::time::sleep(poll).await;
    }
}

/// Dispatch a single job by type. `Ok(Some(result))` records ids back on the job (e.g. the posted
/// message id); `Err(msg)` nacks it for retry/park.
async fn perform_job(
    api: &OisClient,
    http: &Arc<Http>,
    job: &OutboundJob,
) -> Result<Option<Value>, String> {
    match job.job_type.as_str() {
        "ace_request_post" => post_ace_request(http, &job.payload).await,
        "ace_request_notify" => notify_ace_claim(http, &job.payload).await,
        "tmi_publish" => post_tmi(http, &job.payload).await,
        "event_thread_create" => create_event_thread(http, &job.payload).await,
        // The admin's "Refresh from Discord" button — re-pull + push the guild snapshot.
        "guild_snapshot" => snapshot_and_push(http, api).await.map(|()| None),
        other => Err(format!("unknown job type: {other}")),
    }
}

/// Convert a serenity `ChannelType` to the short kind string stored in the snapshot.
fn channel_kind(k: ChannelType) -> &'static str {
    match k {
        ChannelType::Text => "text",
        ChannelType::Voice => "voice",
        ChannelType::Category => "category",
        ChannelType::News => "announcement",
        ChannelType::Forum => "forum",
        ChannelType::Stage => "stage",
        ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread => {
            "thread"
        }
        _ => "other",
    }
}

/// Pull every guild the bot is in (its channels + roles) via REST and push the full snapshot to the
/// backend, so the admin config can offer dropdowns.
async fn snapshot_and_push(http: &Arc<Http>, api: &OisClient) -> Result<(), String> {
    let guilds = http
        .get_guilds(None, None)
        .await
        .map_err(|e| format!("get_guilds: {e}"))?;
    let mut out = Vec::with_capacity(guilds.len());
    for gi in guilds {
        let gid = gi.id;
        let channels = gid
            .channels(http)
            .await
            .map_err(|e| format!("channels({gid}): {e}"))?;
        let roles = gid
            .roles(http)
            .await
            .map_err(|e| format!("roles({gid}): {e}"))?;
        out.push(GuildSnap {
            guild_id: gid.get().to_string(),
            name: gi.name.clone(),
            channels: channels
                .values()
                .map(|c| GuildChannelSnap {
                    id: c.id.get().to_string(),
                    name: c.name.clone(),
                    kind: channel_kind(c.kind).to_string(),
                    parent_id: c.parent_id.map(|p| p.get().to_string()),
                    position: c.position as i32,
                })
                .collect(),
            roles: roles
                .values()
                .map(|r| GuildRoleSnap {
                    id: r.id.get().to_string(),
                    name: r.name.clone(),
                    managed: r.managed,
                    position: r.position as i32,
                })
                .collect(),
        });
    }
    let count = out.len();
    api.push_guild_snapshot(out)
        .await
        .map_err(|e| format!("push: {e}"))?;
    tracing::info!(guilds = count, "pushed guild snapshot");
    Ok(())
}

/// Extract "HHMM" from an ISO-8601 timestamp (e.g. `2026-08-23T23:30:00+00:00` → `2330`).
fn hhmm(iso: &str) -> String {
    iso.split('T')
        .nth(1)
        .map(|t| t.chars().take(5).filter(char::is_ascii_digit).collect())
        .unwrap_or_default()
}

/// The ACE request embed: details + an "X/N claimed" slot meter + the claimer lines. Shared by the
/// initial post (0/N) and the claim/release notify (edits the same message).
fn ace_embed(p: &Value, claims_count: i64, filled: bool) -> CreateEmbed {
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

async fn post_ace_request(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let request_id = str_field(p, "request_id").ok_or("missing request_id")?;
    let button = CreateButton::new(format!("{ACE_CLAIM_PREFIX}{request_id}"))
        .label("Claim")
        .style(ButtonStyle::Primary);
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

async fn notify_ace_claim(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
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
        let button = CreateButton::new(format!("{ACE_CLAIM_PREFIX}{request_id}"))
            .label("Claim")
            .style(ButtonStyle::Primary);
        vec![CreateActionRow::Buttons(vec![button])]
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

async fn post_tmi(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
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

const DIVIDER: &str = "───────────────────────────";

async fn create_event_thread(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let title = str_field(p, "event_title").unwrap_or("Event coordination");
    let thread_name = str_field(p, "thread_name").unwrap_or(title);
    let date_line = str_field(p, "date_line").unwrap_or("");

    let thread = channel
        .create_thread(
            http,
            CreateThread::new(truncate(thread_name, 100)).kind(ChannelType::PublicThread),
        )
        .await
        .map_err(|e| format!("create_thread failed: {e}"))?;

    // Facility lines: ping each facility's EC(s) — the OIS users holding the `EC` role scoped to that
    // ARTCC (from Access Control), by their linked Discord id. NTMO/DCC-Trainee stay config roles.
    let parse_role = |v: Option<&str>| v.and_then(|s| s.parse::<u64>().ok()).map(RoleId::new);
    let mut role_ids: Vec<RoleId> = Vec::new();
    let mut user_ids: Vec<UserId> = Vec::new();
    let mut facility_lines = String::new();
    if let Some(facs) = p.get("facilities").and_then(Value::as_array) {
        for f in facs {
            let id = f.get("id").and_then(Value::as_str).unwrap_or("?");
            let ecs: Vec<u64> = f
                .get("ec_user_ids")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter_map(|s| s.parse::<u64>().ok())
                        .collect()
                })
                .unwrap_or_default();
            let ping = if ecs.is_empty() {
                " _(no EC assigned)_".to_string()
            } else {
                ecs.iter().map(|u| format!(" <@{u}>")).collect()
            };
            for u in &ecs {
                user_ids.push(UserId::new(*u));
            }
            facility_lines.push_str(&format!("• **{id}**{ping}\n"));
        }
    }
    if facility_lines.is_empty() {
        facility_lines.push_str("_No facilities marked required/preferred yet._\n");
    }

    let ntmo = str_field(p, "ntmo_role_id");
    let dcc = str_field(p, "dcc_trainee_role_id");
    for r in [parse_role(ntmo), parse_role(dcc)].into_iter().flatten() {
        role_ids.push(r);
    }
    let ntmo_ping = ntmo
        .map(|r| format!("<@&{r}>"))
        .unwrap_or_else(|| "@NTMO".to_string());
    let dcc_ping = dcc
        .map(|r| format!("<@&{r}>"))
        .unwrap_or_else(|| "@DCC Trainee".to_string());

    let content = format!(
        "**{title} | Planning Thread**\n\
         {title} is on {date_line}\n\n\
         Review the following for your facility:\n\
         - TMU/TMI package\n\
         - Staffing\n\
         - Configs and AAR\n\n\
         {facility_lines}\n\
         Attempt to coordinate as many plans (initiatives, reroutes, etc.) in a timely manner, and \
         fill out all appropriate areas of the staffing data.\n\
         {DIVIDER}\n\
         {ntmo_ping} please react with your availability to NOM for this event. {dcc_ping} please \
         react with your availability to shadow this event.\n\n\
         🟢 = Available\n🟡 = Partially available/unsure\n🔴 = Unavailable\n\
         {DIVIDER}"
    );

    let event_id = p
        .get("event_id")
        .and_then(Value::as_i64)
        .map(|n| n.to_string())
        .unwrap_or_default();
    let buttons = vec![
        CreateButton::new(format!("evtavail:green:{event_id}"))
            .emoji('🟢')
            .style(ButtonStyle::Success),
        CreateButton::new(format!("evtavail:yellow:{event_id}"))
            .emoji('🟡')
            .style(ButtonStyle::Secondary),
        CreateButton::new(format!("evtavail:red:{event_id}"))
            .emoji('🔴')
            .style(ButtonStyle::Danger),
    ];

    // Discord rejects duplicate ids in allowed_mentions (an EC can cover several facilities; roles
    // can repeat too).
    role_ids.sort();
    role_ids.dedup();
    user_ids.sort();
    user_ids.dedup();
    let message = CreateMessage::new()
        .content(content)
        .allowed_mentions(CreateAllowedMentions::new().roles(role_ids).users(user_ids))
        .components(vec![CreateActionRow::Buttons(buttons)]);
    thread
        .id
        .send_message(http, message)
        .await
        .map_err(|e| format!("thread send_message failed: {e}"))?;
    Ok(Some(json!({ "thread_id": thread.id.get().to_string() })))
}

/// Resolve the `channel_id` snowflake from a job payload.
fn channel(p: &Value) -> Result<ChannelId, String> {
    let id: u64 = str_field(p, "channel_id")
        .ok_or("missing channel_id")?
        .parse()
        .map_err(|_| "channel_id not a snowflake".to_string())?;
    Ok(ChannelId::new(id))
}

fn str_field<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Clamp to `max` characters (Discord thread names cap at 100), on a char boundary.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}
