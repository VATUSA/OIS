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
//! - notifications, tray, hotkeys and the rest — #348-#354

// Release builds on Windows are GUI apps, so suppress the console window that would otherwise
// appear behind them. Debug builds keep it — that's where our logs go.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;

fn main() {
    tauri::Builder::default()
        // Signed auto-update (#347). The plugin checks the endpoint in tauri.conf.json and will not
        // apply a package whose signature doesn't verify against the configured public key; the
        // frontend drives when that happens (`web/src/lib/desktop-update.ts`) so the app never
        // restarts itself out from under someone mid-event.
        // Native notifications (#348). The frontend decides what is worth notifying about and
        // whether the user asked for it; this is delivery.
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            auth::store_token,
            auth::get_token,
            auth::delete_token,
            auth::begin_login,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the OIS desktop shell");
}
