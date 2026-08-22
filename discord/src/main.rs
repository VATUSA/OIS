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

use ois_client::{OisClient, OutboundJob};
use serde_json::{Value, json};
use serenity::all::{
    ButtonStyle, ChannelId, Colour, Context, CreateActionRow, CreateButton, CreateEmbed,
    CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, EditMessage,
    EventHandler, GatewayIntents, Http, Interaction, MessageId, Ready,
};

const ACE_CLAIM_PREFIX: &str = "ace_claim:";

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
    async fn ready(&self, _ctx: Context, ready: Ready) {
        tracing::info!(bot = %ready.user.name, "discord bot connected");
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Component(mc) = interaction else {
            return;
        };
        let Some(request_id) = mc.data.custom_id.strip_prefix(ACE_CLAIM_PREFIX) else {
            return;
        };
        let discord_user = mc.user.id.get().to_string();
        let content = match self
            .api
            .claim_ace_via_discord(request_id, &discord_user)
            .await
        {
            Ok(_) => "✅ Claimed — thanks for covering this.".to_string(),
            Err(e) => match e.status() {
                Some(403) => {
                    "No OIS account is linked to your Discord. Add your Discord to your VATUSA \
                     profile, then sign in to OIS to sync it."
                        .to_string()
                }
                Some(409) => "Someone already claimed this request.".to_string(),
                Some(404) => "That request no longer exists.".to_string(),
                _ => {
                    tracing::error!(error = %e, request_id, "ace claim callback failed");
                    "Couldn’t claim right now — please try again.".to_string()
                }
            },
        };
        let response = CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(content)
                .ephemeral(true),
        );
        if let Err(e) = mc.create_response(&ctx.http, response).await {
            tracing::error!(error = %e, "failed to respond to claim interaction");
        }
    }
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
                    match perform_job(&http, &job).await {
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
async fn perform_job(http: &Arc<Http>, job: &OutboundJob) -> Result<Option<Value>, String> {
    match job.job_type.as_str() {
        "ace_request_post" => post_ace_request(http, &job.payload).await,
        "ace_request_notify" => notify_ace_claim(http, &job.payload).await,
        "tmi_publish" => post_tmi(http, &job.payload).await,
        other => Err(format!("unknown job type: {other}")),
    }
}

async fn post_ace_request(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let mut embed = CreateEmbed::new()
        .title("ACE coverage request")
        .colour(Colour::new(0x5865F2));
    if let Some(artcc) = str_field(p, "artcc_id") {
        embed = embed.field("ARTCC", artcc, true);
    }
    if let Some(position) = str_field(p, "position") {
        embed = embed.field("Position", position, true);
    }
    if let Some(by) = str_field(p, "requested_by_name") {
        embed = embed.field("Requested by", by, true);
    }
    if let Some(details) = str_field(p, "details") {
        embed = embed.description(details);
    }
    let request_id = str_field(p, "request_id").ok_or("missing request_id")?;
    let button = CreateButton::new(format!("{ACE_CLAIM_PREFIX}{request_id}"))
        .label("Claim")
        .style(ButtonStyle::Primary);
    let message = channel
        .send_message(
            http,
            CreateMessage::new()
                .embed(embed)
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
    let claimed_by = str_field(p, "claimed_by_name").unwrap_or("a controller");
    let embed = CreateEmbed::new()
        .title("ACE coverage request — claimed")
        .colour(Colour::new(0x57F287))
        .field("Claimed by", claimed_by, true);
    // Drop the button now that it's claimed.
    channel
        .edit_message(
            http,
            MessageId::new(message_id),
            EditMessage::new().embed(embed).components(vec![]),
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
