//! The OIS desktop shell.
//!
//! This is a Tauri window around the **existing `web/` SPA** — not a second frontend. The same
//! bundle that serves the website renders here (`frontendDist` in `tauri.conf.json` points at
//! `web/`'s Vite output), so `@ois/ui`, `@ois/api-client` and all of `web/src` ship unchanged and
//! the desktop app is literally the same app.
//!
//! What it adds beyond the window is authentication (see [`auth`]): the desktop app can't carry the
//! website's session cookie, so it holds a session token in the OS keychain and sends it as a
//! bearer. The remaining desktop features arrive in later issues, each adding its own commands and
//! capability entries:
//!
//! - native notifications whose click opens the page they're about — #348 (see [`notify`])
//! - tray, hotkeys and the rest — #349-#354

// Release builds on Windows are GUI apps, so suppress the console window that would otherwise
// appear behind them. Logs go to the log file either way (see `logging`); debug builds also print them
// to this console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod diagnostics;
mod logging;
mod notify;
mod popout;
mod redact;
mod window_shape;

fn main() {
    logging::log_panics();
    tauri::Builder::default()
        // The log file (#629). First, so everything after it can log.
        .plugin(logging::plugin())
        // Launch at login (#351). Registered always; whether it is *enabled* is this computer's
        // login item, read and written from the settings page — never an account setting.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // Global shortcuts (#352). Registration is driven from the frontend, where the user's
        // configured accelerators live.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Signed auto-update (#347). The plugin checks the endpoint in tauri.conf.json and will not
        // apply a package whose signature doesn't verify against the configured public key; the
        // frontend drives when that happens (`web/src/lib/desktop-update.ts`) so the app never
        // restarts itself out from under someone mid-event.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Native notifications (#348). The frontend decides what is worth notifying about and
        // whether the user asked for it. The plugin is here for the OS permission prompt; raising
        // a notification goes through `notify::notify`, because only that reports the click.
        .plugin(tauri_plugin_notification::init())
        // Pop-outs and route windows close with the main window rather than outliving it (#349, #350).
        .on_window_event(popout::on_window_event)
        .invoke_handler(tauri::generate_handler![
            auth::store_token,
            auth::get_token,
            auth::delete_token,
            auth::begin_login,
            notify::notify,
            diagnostics::send_diagnostics,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start the OIS desktop shell")
        .run(|_app, _event| {
            // macOS keeps an app running with no windows, so closing the window (or hiding it to
            // the menu bar) leaves it alive in the dock with no way back in — clicking the dock
            // icon is the way back (#351).
            //
            // macOS-only in the literal sense: `RunEvent::Reopen` does not exist on Windows or
            // Linux, so it must be compiled out there rather than merely skipped at runtime — a
            // plain runtime check does not compile at all (caught by the 3-OS CI matrix).
            //
            // Nested rather than a let chain: those need Rust 1.88, and the workspace promises 1.85.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                if let Some(window) = tauri::Manager::get_webview_window(_app, "main") {
                    // A failure here leaves the user with no way back into the app, so it is logged.
                    if let Err(e) = window.show().and_then(|()| window.set_focus()) {
                        log::warn!("could not bring the main window back on reopen: {e}");
                    }
                }
            }
        });
}
