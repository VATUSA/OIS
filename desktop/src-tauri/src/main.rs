//! The OIS desktop shell.
//!
//! This is a Tauri window around the **existing `web/` SPA** — not a second frontend. The same
//! bundle that serves the website renders here (`frontendDist` in `tauri.conf.json` points at
//! `web/`'s Vite output), so `@ois/ui`, `@ois/api-client` and all of `web/src` ship unchanged and
//! the desktop app is literally the same app.
//!
//! It is deliberately bare. The pieces that make it *desktop* arrive in later issues and each one
//! adds its own plugins, commands and capability entries:
//!
//! - platform capability layer / IPC conventions — #345
//! - keychain-stored auth token — #346
//! - distribution + signed auto-update — #347
//!
//! Until then: one window, no commands, no plugins.

// Release builds on Windows are GUI apps, so suppress the console window that would otherwise
// appear behind them. Debug builds keep it — that's where our logs go.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to start the OIS desktop shell");
}
