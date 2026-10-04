//! "Send diagnostics" (#629): a report the user chooses to send, built and uploaded from Rust.
//!
//! The webview supplies what only it knows (route, window, capabilities, realtime history, its own log
//! tail) and a note from the user. Rust adds the platform facts, reads the rolled log files, redacts
//! everything again — a file written by an older build was never redacted at write time — gzips the
//! logs and posts them with the session token from its own store. Neither the log files nor the token
//! cross into JavaScript for this.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::{logging::LOG_FILE_NAME, redact::redact};

/// The server's route; it only accepts a desktop session token (`ois_dsk_…`).
const UPLOAD_PATH: &str = "/api/v1/diagnostics/reports";

/// The most log text one report carries, from the end of the newest file backwards. The files on disk
/// top out near 8 MB (see `logging.rs`); text compresses roughly tenfold, well inside the 5 MB cap.
const MAX_LOG_BYTES: usize = 12 * 1024 * 1024;

/// Uploads one report and returns its id, or a message fit to show the user.
#[tauri::command]
pub async fn send_diagnostics(
    app: AppHandle,
    api_base: String,
    note: String,
    context: Value,
) -> Result<String, String> {
    let token = crate::auth::get_token(app.clone())?
        .ok_or_else(|| "Sign in to send diagnostics.".to_string())?;
    let logs = match app.path().app_log_dir() {
        Ok(dir) => read_logs(&dir),
        Err(e) => {
            log::warn!("diagnostics: no log directory: {e}");
            String::new()
        }
    };
    let meta = json!({
        "app_version": app.package_info().version.to_string(),
        "bundle_id": app.config().identifier,
        "os": std::env::consts::OS,
        "os_version": os_version(),
        "arch": std::env::consts::ARCH,
        "webview_version": tauri::webview_version().unwrap_or_default(),
        "note": note,
        "context": context,
    });
    let (meta, logs) = build_payload(&meta, &logs)?;

    let form = reqwest::multipart::Form::new()
        .part(
            "meta",
            reqwest::multipart::Part::text(meta)
                .mime_str("application/json")
                .map_err(|e| e.to_string())?,
        )
        .part(
            "logs",
            reqwest::multipart::Part::bytes(logs)
                .file_name("logs.gz")
                .mime_str("application/gzip")
                .map_err(|e| e.to_string())?,
        );
    let url = format!("{}{UPLOAD_PATH}", api_base.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            log::warn!("diagnostics: upload failed: {e}");
            "Couldn't reach OIS to send diagnostics. Check your connection and try again."
                .to_string()
        })?;

    let status = response.status();
    if status.is_success() {
        let body: Value = response.json().await.unwrap_or_default();
        let id = body["id"].as_str().unwrap_or_default().to_string();
        log::info!("diagnostics: sent report {id}");
        return Ok(id);
    }
    log::warn!("diagnostics: upload refused with {status}");
    Err(match status.as_u16() {
        401 => "Your session has expired. Sign in again, then send diagnostics.".into(),
        413 => "The report is too large to send.".into(),
        429 => "You've sent several reports recently. Please wait an hour and try again.".into(),
        _ => format!("OIS couldn't accept the report ({status})."),
    })
}

/// The `meta` JSON and the gzipped logs, both redacted, the logs cut to their last
/// [`MAX_LOG_BYTES`]. Redacted *before* the cut, so a cut can never leave half a token behind for the
/// patterns to miss. Pure, so what is actually sent is testable without a network or an app.
pub fn build_payload(meta: &Value, logs: &str) -> Result<(String, Vec<u8>), String> {
    let meta = redact(&meta.to_string());
    let logs = tail(redact(logs), MAX_LOG_BYTES);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(logs.as_bytes())
        .and_then(|()| encoder.finish())
        .map(|logs| (meta, logs))
        .map_err(|e| format!("could not compress the logs: {e}"))
}

/// The OIS log files in `dir`, oldest first, joined with a header per file.
fn read_logs(dir: &Path) -> String {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(LOG_FILE_NAME)
        })
        .filter_map(|entry| {
            let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    files.sort();
    let mut text = String::new();
    for (_, path) in files {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                text.push_str(&format!("===== {name} =====\n"));
                text.push_str(&String::from_utf8_lossy(&bytes));
            }
            Err(e) => log::warn!("diagnostics: could not read {}: {e}", path.display()),
        }
    }
    text
}

/// The last `max` bytes of `text`, cut on a character boundary.
fn tail(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// The OS version, best effort: an empty string rather than a failed report.
fn os_version() -> String {
    #[cfg(target_os = "macos")]
    let version = command_output("sw_vers", &["-productVersion"]);
    #[cfg(windows)]
    let version = command_output("cmd", &["/C", "ver"]);
    #[cfg(not(any(target_os = "macos", windows)))]
    let version = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|release| {
            release
                .lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|name| name.trim_matches('"').to_string())
        });
    version.unwrap_or_default()
}

#[cfg(any(target_os = "macos", windows))]
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    // The release build is a GUI app; spawning `cmd` from it would flash a console window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;

    use super::*;

    const TOKEN: &str = "ois_dsk_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn gunzip(bytes: &[u8]) -> String {
        let mut text = String::new();
        GzDecoder::new(bytes).read_to_string(&mut text).unwrap();
        text
    }

    /// #629 AC5: whatever an old build wrote to disk, and whatever the user pastes into the note, the
    /// bytes that leave the machine carry no token.
    #[test]
    fn the_sent_payload_carries_no_token() {
        let meta = json!({ "note": format!("it broke, my token is {TOKEN}"), "context": { "route": "/flow" } });
        let logs = format!("old build line: Authorization: Bearer {TOKEN}\n");

        let (meta, logs) = build_payload(&meta, &logs).unwrap();

        let suffix = &TOKEN["ois_dsk_".len()..];
        assert!(!meta.contains(suffix), "{meta}");
        let logs = gunzip(&logs);
        assert!(!logs.contains(suffix), "{logs}");
        let meta: Value = serde_json::from_str(&meta).expect("still JSON after redaction");
        assert_eq!(meta["context"]["route"], "/flow");
    }

    /// The cut comes after redaction. Cut first, a token straddling the cut would leave its last 20
    /// hex characters behind — too short for the 32-hex backstop, so they would be sent.
    #[test]
    fn a_token_at_the_cut_is_redacted_before_the_cut() {
        let logs = format!("{TOKEN}{}", "x".repeat(MAX_LOG_BYTES - 20));
        let (_, gz) = build_payload(&json!({}), &logs).unwrap();
        let sent = gunzip(&gz);
        assert!(sent.len() <= MAX_LOG_BYTES);
        assert!(
            !sent.contains("89abcdef"),
            "no fragment of the token survives"
        );
    }

    #[test]
    fn the_log_tail_keeps_the_end_and_a_whole_character() {
        assert_eq!(tail("abcdef".into(), 3), "def");
        assert_eq!(tail("ab".into(), 3), "ab");
        assert_eq!(tail("aé".into(), 1), "", "never splits a character");
    }
}
