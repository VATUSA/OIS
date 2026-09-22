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
//! - distribution + signed auto-update — #347
//! - notifications, tray, hotkeys and the rest — #348-#354

// Release builds on Windows are GUI apps, so suppress the console window that would otherwise
// appear behind them. Debug builds keep it — that's where our logs go.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            auth::store_token,
            auth::get_token,
            auth::delete_token,
            auth::begin_login,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the OIS desktop shell");
}
