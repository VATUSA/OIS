use std::time::Duration;

pub(crate) struct Config {
    pub(crate) discord_token: String,
    pub(crate) api_base: String,
    pub(crate) api_token: String,
    pub(crate) poll: Duration,
}

impl Config {
    pub(crate) fn from_env() -> Result<Self, String> {
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
