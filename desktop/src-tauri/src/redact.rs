//! Secret redaction for everything the desktop app writes to its log or sends in a diagnostics report
//! (#629).
//!
//! The desktop session token is handed to the webview (`auth::get_token`) and travels as a bearer, so
//! any line that echoes a request, a header or an error can carry a live credential. A log file is an
//! incidental store like a URL: the token must never reach it. Every log record passes through
//! [`redact`] in the logger's formatter, and a diagnostics bundle is redacted again before upload, so a
//! file written by an older build is covered too.
//!
//! Redaction errs toward over-matching: a hex id lost from a log line costs little, a leaked token a
//! great deal. No pattern consumes a quote, so redacting serialized JSON leaves it valid JSON.

use std::sync::LazyLock;

use regex::Regex;

/// What a secret is replaced with.
pub const REDACTED: &str = "[redacted]";

struct Rule {
    pattern: Regex,
    replacement: &'static str,
}

fn rule(pattern: &str, replacement: &'static str) -> Rule {
    Rule {
        pattern: Regex::new(pattern).expect("redaction patterns are fixed and valid"),
        replacement,
    }
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        // OIS credentials by prefix: desktop sessions, API keys, service accounts. Also catches the
        // websocket subprotocol form `ois.bearer.ois_dsk_…`.
        rule(r"(ois_(?:dsk|pat|sa)_)[A-Za-z0-9_\-]+", "${1}[redacted]"),
        // Bearer credentials of any shape, e.g. a logged `Authorization` header or METRICS_TOKEN.
        rule(r#"(?i)(bearer\s+)[^\s,;"']+"#, "${1}[redacted]"),
        rule(
            r#"(?i)(metrics_token\s*[=:]\s*)[^\s,;"']+"#,
            "${1}[redacted]",
        ),
        // The one-time desktop sign-in code and its state nonce: no prefix, so by key — as a query
        // parameter (`/callback?code=…&state=…`) and as JSON (the exchange body `{"code":"…"}`).
        rule(
            r#"(?i)\b((?:desktop_state|state|code)=)[^&\s"']+"#,
            "${1}[redacted]",
        ),
        rule(
            r#"(?i)("(?:code|state|token)"\s*:\s*")[^"]*(")"#,
            "${1}[redacted]${2}",
        ),
        // Backstop: any long hex run (session tokens, codes and nonces are 64 hex characters).
        rule(r"\b[0-9a-fA-F]{32,}\b", REDACTED),
    ]
});

/// `text` with every secret it recognises replaced by [`REDACTED`].
pub fn redact(text: &str) -> String {
    RULES.iter().fold(text.to_owned(), |acc, rule| {
        rule.pattern
            .replace_all(&acc, rule.replacement)
            .into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_desktop_session_token_never_survives() {
        let token = format!("ois_dsk_{HEX}");
        let line = format!("GET /api/v1/me failed: Authorization: Bearer {token} -> 401");
        let out = redact(&line);
        assert!(!out.contains(HEX), "{out}");
        assert!(out.contains("Bearer [redacted]"), "{out}");
        // Bare, as `get_token` would hand it over, and inside the websocket subprotocol offer.
        assert_eq!(redact(&token), "ois_dsk_[redacted]");
        assert_eq!(
            redact(&format!("protocols: ois.v1, ois.bearer.{token}")),
            "protocols: ois.v1, ois.bearer.ois_dsk_[redacted]"
        );
    }

    #[test]
    fn api_key_and_service_account_secrets_never_survive() {
        assert_eq!(redact("key ois_pat_abc123XYZ"), "key ois_pat_[redacted]");
        assert_eq!(redact("sa=ois_sa_Zz-9_8"), "sa=ois_sa_[redacted]");
    }

    /// The one-time sign-in code crosses `127.0.0.1:8765` in plain text and has no prefix.
    #[test]
    fn the_one_time_auth_code_and_its_state_never_survive() {
        let request = format!("GET /callback?code={HEX}&state={HEX} HTTP/1.1");
        assert_eq!(
            redact(&request),
            "GET /callback?code=[redacted]&state=[redacted] HTTP/1.1"
        );
        assert_eq!(
            redact(&format!(r#"exchange body {{"code":"{HEX}"}}"#)),
            r#"exchange body {"code":"[redacted]"}"#
        );
        assert_eq!(
            redact("desktop_state=abc&return_to=x"),
            "desktop_state=[redacted]&return_to=x"
        );
    }

    #[test]
    fn metrics_token_never_survives() {
        assert_eq!(
            redact("METRICS_TOKEN=s3cr3t-value other"),
            "METRICS_TOKEN=[redacted] other"
        );
        assert_eq!(
            redact("metrics_token: hunter2"),
            "metrics_token: [redacted]"
        );
    }

    #[test]
    fn a_long_hex_run_is_redacted_even_without_a_key() {
        assert_eq!(
            redact(&format!("nonce {HEX} seen")),
            "nonce [redacted] seen"
        );
    }

    #[test]
    fn redacting_json_keeps_it_json() {
        let json = format!(r#"{{"auth":"Bearer ois_dsk_{HEX}","route":"/flow"}}"#);
        let value: serde_json::Value = serde_json::from_str(&redact(&json)).unwrap();
        assert_eq!(value["auth"], "Bearer [redacted]");
        assert_eq!(value["route"], "/flow");
    }

    #[test]
    fn ordinary_lines_pass_through() {
        let line = "realtime connected to /api/v1/ws (retry 2), route /flow/fca/12";
        assert_eq!(redact(line), line);
    }
}
