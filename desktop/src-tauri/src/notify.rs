//! Native notifications that report their click back to the app (#348).
//!
//! `tauri-plugin-notification` 2.4 can't do this on desktop: its desktop `show()` hands notify-rust
//! only a title, body, icon and sound — the route in `extra` is dropped — and nothing on desktop
//! ever emits the `actionPerformed` event its `onAction` listens for. A notification raised through
//! it can't take anyone anywhere. So this raises the notification through each platform's own API,
//! with that platform's click callback, and on a click brings the main window forward and emits
//! [`CLICK_EVENT`] carrying the route the frontend attached (`web/src/lib/desktop-notify.ts`).
//!
//! The plugin stays registered for the OS permission prompt, and as the fallback on platforms with
//! no click support here.

use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The event a click emits, with the notification's in-app route as its payload.
///
/// Only the macOS and Windows branches below report a click, so on any other platform this and
/// [`on_click`] are unreachable from the binary — dead code that `-D warnings` rejects, which macOS
/// clippy cannot see because it compiles a different branch. Both stay compiled in regardless: the
/// test that pins this event name as the contract with `web/src/lib/desktop-notify.ts` has to run on
/// Linux too, which is where CI runs the Rust job.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
pub const CLICK_EVENT: &str = "notification-clicked";

/// Raises a native notification; clicking it raises the app and emits [`CLICK_EVENT`] with `route`.
#[tauri::command]
pub fn notify(app: AppHandle, title: String, body: String, route: String) -> Result<(), String> {
    platform::show(app, title, body, route)
}

/// What a click does on every platform: bring the main window to the front, then hand the route to
/// the frontend. Unminimise *and* show *and* focus — a backgrounded app needs a different one of
/// those on each platform, and doing all three is the only reliable way to end up in front.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn on_click<R: Runtime>(app: &AppHandle<R>, route: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let raised = window
            .unminimize()
            .and_then(|()| window.show())
            .and_then(|()| window.set_focus());
        if let Err(e) = raised {
            log::warn!("notification click could not bring the main window forward: {e}");
        }
    }
    // The route is an in-app path, safe to log; without the event the click silently goes nowhere.
    if let Err(e) = app.emit(CLICK_EVENT, route) {
        log::error!("notification click for {route} was not delivered: {e}");
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mac_notification_sys::{Notification, NotificationResponse};
    use tauri::AppHandle;

    /// macOS reports a click by blocking a thread until the notification is clicked or cleared, so
    /// every notification still sitting in Notification Center holds one. Past this many, new ones
    /// still show but aren't clickable, rather than growing threads without bound for someone who
    /// never clears them.
    pub(super) const MAX_CLICKABLE: usize = 32;
    static CLICKABLE: AtomicUsize = AtomicUsize::new(0);

    /// One of the [`MAX_CLICKABLE`] waiting threads, released when dropped.
    pub(super) struct ClickSlot;

    impl ClickSlot {
        pub(super) fn claim() -> Option<Self> {
            if CLICKABLE.fetch_add(1, Ordering::SeqCst) < MAX_CLICKABLE {
                Some(ClickSlot)
            } else {
                CLICKABLE.fetch_sub(1, Ordering::SeqCst);
                None
            }
        }
    }

    impl Drop for ClickSlot {
        fn drop(&mut self) {
            CLICKABLE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub fn show(app: AppHandle, title: String, body: String, route: String) -> Result<(), String> {
        // Which app the notification is attributed to. Errors once already set — by an earlier
        // notification or by the plugin — which is fine. In dev there is no bundle to point at, so
        // borrow Terminal's, as the plugin does.
        let bundle = if tauri::is_dev() {
            "com.apple.Terminal".to_owned()
        } else {
            app.config().identifier.clone()
        };
        let _ = mac_notification_sys::set_application(&bundle);

        let slot = ClickSlot::claim();
        std::thread::spawn(move || {
            let mut options = Notification::new();
            options.wait_for_click(slot.is_some());
            let response =
                mac_notification_sys::send_notification(&title, None, &body, Some(&options));
            drop(slot);
            if let Ok(NotificationResponse::Click) = response {
                super::on_click(&app, &route);
            }
        });
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use tauri::AppHandle;
    use tauri_winrt_notification::Toast;

    pub fn show(app: AppHandle, title: String, body: String, route: String) -> Result<(), String> {
        // The installed app has its own AppUserModelID; a bare `target\debug` binary doesn't, and a
        // toast under an unregistered ID is silently dropped — so borrow PowerShell's there, as the
        // plugin does.
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.display().to_string()))
            .unwrap_or_default();
        let unpackaged = exe_dir.ends_with(r"target\debug") || exe_dir.ends_with(r"target\release");
        let app_id = if unpackaged {
            Toast::POWERSHELL_APP_ID.to_owned()
        } else {
            app.config().identifier.clone()
        };

        let handle = app.clone();
        Toast::new(&app_id)
            .title(&title)
            .text1(&body)
            .on_activated(move |_| {
                super::on_click(&handle, &route);
                Ok(())
            })
            .show()
            .map_err(|e| format!("could not show the notification: {e}"))
    }
}

/// Anywhere else (Linux): the plugin's notification, which shows but reports no click.
#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use tauri::AppHandle;
    use tauri_plugin_notification::NotificationExt;

    pub fn show(app: AppHandle, title: String, body: String, _route: String) -> Result<(), String> {
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| format!("could not show the notification: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use tauri::Listener;

    use super::*;

    /// The route is the whole point of the click: it must reach the frontend exactly as attached,
    /// under the event name `desktop-notify.ts` listens for.
    #[test]
    fn a_click_hands_the_route_to_the_frontend() {
        let app = tauri::test::mock_app();
        let (tx, rx) = mpsc::channel();
        // The literal, not `CLICK_EVENT`: this string is the contract with `desktop-notify.ts`, and
        // renaming the constant alone must fail here rather than silently stop every click.
        app.listen("notification-clicked", move |event| {
            tx.send(event.payload().to_owned()).unwrap();
        });

        on_click(app.handle(), "/ops/fca?fca=ZDC-1");

        let payload = rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        assert_eq!(payload, "\"/ops/fca?fca=ZDC-1\"");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clickable_notifications_are_capped_and_a_cleared_one_frees_its_slot() {
        use super::platform::{ClickSlot, MAX_CLICKABLE};

        let held: Vec<_> = (0..MAX_CLICKABLE).map(|_| ClickSlot::claim()).collect();
        assert!(held.iter().all(Option::is_some));
        assert!(
            ClickSlot::claim().is_none(),
            "one past the cap is shown, not clickable"
        );

        drop(held);
        assert!(
            ClickSlot::claim().is_some(),
            "cleared notifications give their threads back"
        );
    }
}
