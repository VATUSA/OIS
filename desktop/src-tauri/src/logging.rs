//! The desktop log file (#629).
//!
//! One rotating file in the OS log directory, holding both halves of the app: Rust's own `log` records
//! and the webview's, which arrive through the plugin's `log` command as records targeted `webview:…`
//! (`web/src/lib/logger.ts`). Every line goes through [`format_line`], and so through
//! [`crate::redact::redact`], before anything is written.
//!
//! | OS      | Directory                                  |
//! | ------- | ------------------------------------------ |
//! | macOS   | `~/Library/Logs/net.vatusa.ois/`           |
//! | Windows | `%LOCALAPPDATA%\net.vatusa.ois\logs\`      |
//! | Linux   | `~/.local/share/net.vatusa.ois/logs/`      |

use tauri::{Runtime, plugin::TauriPlugin};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

use crate::redact::redact;

/// The log file's base name; rotated files keep it as a prefix (`ois_<date>.log`).
pub const LOG_FILE_NAME: &str = "ois";

/// Each file is cut at 2 MB and the three most recent are kept, so the log never holds more than about
/// 8 MB — comfortably under the diagnostics upload cap once compressed.
const MAX_FILE_BYTES: u128 = 2 * 1024 * 1024;
const KEEP_ROTATED: usize = 3;

/// The logger plugin. Registered first so every later plugin's setup can already log.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets([
            Target::new(TargetKind::LogDir {
                file_name: Some(LOG_FILE_NAME.into()),
            }),
            Target::new(TargetKind::Stdout),
        ])
        .max_file_size(MAX_FILE_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(KEEP_ROTATED))
        .level(log::LevelFilter::Info)
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}",
                format_line(
                    &TimezoneStrategy::UseUtc.get_now().to_string(),
                    record.level(),
                    record.target(),
                    &message.to_string(),
                )
            ))
        })
        .build()
}

/// One log line. The message is redacted here, so no target — file, stdout, or a future one — can
/// receive a credential.
pub fn format_line(time: &str, level: log::Level, target: &str, message: &str) -> String {
    format!("{time} [{level}][{target}] {}", redact(message))
}

/// Logs a panic before the default hook prints it, so a crash leaves a line in the file.
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #629 AC5: the formatter is the path every record takes to disk, so a token in any message —
    /// Rust's or the webview's — must come out redacted.
    #[test]
    fn a_line_never_carries_a_desktop_token() {
        let token = "ois_dsk_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let line = format_line(
            "2026-10-03 00:00:00 +00:00:00",
            log::Level::Error,
            "webview:fetch",
            &format!("request failed with token {token}"),
        );
        assert!(!line.contains(&token["ois_dsk_".len()..]), "{line}");
        assert_eq!(
            line,
            "2026-10-03 00:00:00 +00:00:00 [ERROR][webview:fetch] request failed with token ois_dsk_[redacted]"
        );
    }
}
