use std::env::VarError;
use std::time::Duration;

const DEFAULT_POLL_SECS: u64 = 5;

pub(crate) struct Config {
    pub(crate) discord_token: String,
    pub(crate) api_base: String,
    pub(crate) api_token: String,
    pub(crate) poll: Duration,
}

impl Config {
    pub(crate) fn from_env() -> Result<Self, String> {
        Self::from_lookup(|k| std::env::var(k))
    }

    fn from_lookup(lookup: impl Fn(&str) -> Result<String, VarError>) -> Result<Self, String> {
        let get = |k: &str| {
            lookup(k)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("missing required env var {k}"))
        };
        Ok(Self {
            discord_token: get("DISCORD_BOT_TOKEN")?,
            api_base: get("OIS_API_BASE")?,
            api_token: get("OIS_API_TOKEN")?,
            poll: Duration::from_secs(parse_poll_secs(lookup("OIS_POLL_SECS"))?),
        })
    }
}

/// Unset or blank keeps the default; any other value that is not a whole number of seconds
/// above 0 is rejected, so a typo fails startup instead of polling at the default.
fn parse_poll_secs(raw: Result<String, VarError>) -> Result<u64, String> {
    let raw = match raw {
        Err(VarError::NotPresent) => return Ok(DEFAULT_POLL_SECS),
        Err(VarError::NotUnicode(_)) => {
            return Err("invalid OIS_POLL_SECS: the value is not valid UTF-8".to_string());
        }
        Ok(v) => v,
    };
    let value = raw.trim();
    if value.is_empty() {
        return Ok(DEFAULT_POLL_SECS);
    }
    match value.parse::<u64>() {
        Ok(secs) if secs > 0 => Ok(secs),
        _ => Err(format!(
            "invalid OIS_POLL_SECS {value:?}: expected a whole number of seconds above 0"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn lookup_with(poll: Option<&'static str>) -> impl Fn(&str) -> Result<String, VarError> {
        move |k| match k {
            "DISCORD_BOT_TOKEN" => Ok("discord-token".to_string()),
            "OIS_API_BASE" => Ok("http://localhost:8080".to_string()),
            "OIS_API_TOKEN" => Ok("ois-token".to_string()),
            "OIS_POLL_SECS" => poll.map(str::to_string).ok_or(VarError::NotPresent),
            _ => Err(VarError::NotPresent),
        }
    }

    fn assert_rejected(raw: Result<String, VarError>) {
        let err = parse_poll_secs(raw.clone()).expect_err(&format!("{raw:?} must be rejected"));
        assert!(
            err.contains("OIS_POLL_SECS"),
            "error for {raw:?} must name the variable: {err}"
        );
    }

    #[test]
    fn parse_poll_secs_defaults_to_five_when_the_variable_is_absent() {
        assert_eq!(parse_poll_secs(Err(VarError::NotPresent)), Ok(5));
    }

    #[test]
    fn parse_poll_secs_defaults_to_five_when_the_value_is_empty() {
        assert_eq!(parse_poll_secs(Ok(String::new())), Ok(5));
    }

    #[test]
    fn parse_poll_secs_defaults_to_five_when_the_value_is_whitespace() {
        assert_eq!(parse_poll_secs(Ok("  ".to_string())), Ok(5));
    }

    #[test]
    fn parse_poll_secs_returns_a_trimmed_positive_value() {
        assert_eq!(parse_poll_secs(Ok(" 30 ".to_string())), Ok(30));
    }

    #[test]
    fn parse_poll_secs_accepts_one_second() {
        assert_eq!(parse_poll_secs(Ok("1".to_string())), Ok(1));
    }

    #[test]
    fn parse_poll_secs_rejects_a_value_with_a_unit_suffix() {
        assert_rejected(Ok("30s".to_string()));
    }

    #[test]
    fn parse_poll_secs_rejects_zero() {
        assert_rejected(Ok("0".to_string()));
    }

    #[test]
    fn parse_poll_secs_rejects_a_negative_value() {
        assert_rejected(Ok("-1".to_string()));
    }

    #[test]
    fn parse_poll_secs_rejects_a_non_unicode_value() {
        assert_rejected(Err(VarError::NotUnicode(OsString::from("x"))));
    }

    #[test]
    fn from_lookup_fails_naming_ois_poll_secs_when_it_is_malformed() {
        let err = Config::from_lookup(lookup_with(Some("30s")))
            .err()
            .expect("a malformed OIS_POLL_SECS must fail");
        assert!(err.contains("OIS_POLL_SECS"), "{err}");
    }

    #[test]
    fn from_lookup_polls_every_five_seconds_when_ois_poll_secs_is_unset() {
        let config = Config::from_lookup(lookup_with(None)).expect("valid config");
        assert_eq!(config.poll, Duration::from_secs(5));
        assert_eq!(config.discord_token, "discord-token");
        assert_eq!(config.api_base, "http://localhost:8080");
        assert_eq!(config.api_token, "ois-token");
    }

    #[test]
    fn from_lookup_polls_at_the_configured_interval_when_set_to_thirty() {
        let config = Config::from_lookup(lookup_with(Some("30"))).expect("valid config");
        assert_eq!(config.poll, Duration::from_secs(30));
    }

    #[test]
    fn from_lookup_reports_the_missing_required_var_before_a_malformed_poll() {
        let base = lookup_with(Some("30s"));
        let lookup = |k: &str| match k {
            "DISCORD_BOT_TOKEN" => Err(VarError::NotPresent),
            _ => base(k),
        };
        let err = Config::from_lookup(lookup)
            .err()
            .expect("a missing DISCORD_BOT_TOKEN must fail");
        assert_eq!(err, "missing required env var DISCORD_BOT_TOKEN");
    }
}
