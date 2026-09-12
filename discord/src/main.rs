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

mod config;
mod discord;
mod interactions;
mod jobs;
mod snapshot;
mod util;

use ois_client::OisClient;
use serenity::all::GatewayIntents;

use config::Config;
use interactions::Handler;

#[tokio::main]
async fn main() {
    // Pretty, backtrace-rich panic reports (errors here are logged + exit explicitly below).
    let _ = color_eyre::install();
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
    tokio::spawn(jobs::job_loop(api, http, config.poll));

    if let Err(e) = client.start().await {
        tracing::error!(error = %e, "discord gateway error");
        std::process::exit(1);
    }
}
