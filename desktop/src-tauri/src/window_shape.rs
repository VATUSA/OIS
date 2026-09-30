//! The main window's outer shape (#419).
//!
//! The window has no title bar of its own to draw the app's chrome under (#402), and on macOS it is
//! the OS that draws both the traffic lights and the rounded corners — so that platform gets its
//! decorations back through `tauri.macos.conf.json` (`titleBarStyle: "Overlay"`, `hiddenTitle`, and a
//! `trafficLightPosition` that insets the lights into the app's own top bar) and needs nothing here.
//!
//! Windows is the platform that needs code: an undecorated window there is left square by the
//! compositor, so the rounded corners have to be asked for explicitly. Everything in this module is
//! Windows-only for that reason; on every other target [`round_corners`] is a no-op.

/// Ask the compositor to round the window's corners.
///
/// Windows 11 only: `DWMWA_WINDOW_CORNER_PREFERENCE` was added in build 22000, and `DwmSetWindowAttribute`
/// answers `E_INVALIDARG` on Windows 10 — which is why the result is discarded rather than surfaced.
/// A Windows 10 user keeps square corners, and that is the accepted outcome: the alternative is a
/// transparent window with a CSS radius, which costs the native drop shadow and leaves the corners
/// click-through.
///
/// The rounding is the compositor's, so it also clips the webview — nothing in the page needs a
/// matching `border-radius`, and `DESIGN.md`'s "the shell is flush to the viewport" still holds.
#[cfg(windows)]
pub fn round_corners(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };

    let Ok(hwnd) = window.hwnd() else {
        // No handle means no window to shape; the app is still perfectly usable square.
        return;
    };
    let preference = DWMWCP_ROUND;
    // SAFETY: `hwnd` is a live window handle owned by the window we were handed, and the value is a
    // `DWM_WINDOW_CORNER_PREFERENCE` matching the size passed. The call only reads from `preference`.
    unsafe {
        DwmSetWindowAttribute(
            hwnd.0 as _,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &raw const preference as *const _,
            size_of_val(&preference) as u32,
        );
    }
}

/// Non-Windows targets: macOS rounds a decorated window itself, and on Linux it is the compositor's
/// business, not ours.
#[cfg(not(windows))]
pub fn round_corners(_window: &tauri::WebviewWindow) {}

#[cfg(test)]
mod tests {
    /// #419: `tauri.macos.conf.json` exists because `decorations` has to differ per platform for one
    /// window label — macOS needs `true` (an `Overlay` title bar keeps the native traffic lights and
    /// the native corners), Windows and Linux need `false`.
    ///
    /// Tauri merges a platform config over the base with `json_patch::merge`, which **replaces**
    /// arrays rather than merging them element-wise, so the override has to repeat the whole
    /// `app.windows[0]` object — width, minimums, background and all. That duplication is silent: a
    /// size changed in `tauri.conf.json` alone would simply not apply on macOS, and nothing would
    /// fail. This pins the two against each other, so any shared key that drifts fails here instead.
    #[test]
    fn the_macos_window_override_matches_the_base_config() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let macos: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.macos.conf.json"))
                .expect("tauri.macos.conf.json");

        let window = |config: &serde_json::Value| {
            config["app"]["windows"][0]
                .as_object()
                .expect("app.windows[0] is an object")
                .clone()
        };
        // Only `windows[0]` is compared below, so pin the count too: Tauri replaces the whole array,
        // and a second window added to the base alone would simply not exist on macOS (#419 review).
        let count = |config: &serde_json::Value| {
            config["app"]["windows"]
                .as_array()
                .expect("app.windows is an array")
                .len()
        };
        assert_eq!(
            count(&macos),
            count(&base),
            "tauri.macos.conf.json must repeat every window in tauri.conf.json — Tauri replaces the \
             whole array, so one left out here does not exist on macOS at all"
        );

        let (base_window, macos_window) = (window(&base), window(&macos));

        // The keys the override exists to change, plus the macOS-only ones it adds.
        const MACOS_ONLY: [&str; 4] = [
            "decorations",
            "titleBarStyle",
            "hiddenTitle",
            "trafficLightPosition",
        ];

        for (key, value) in &base_window {
            if MACOS_ONLY.contains(&key.as_str()) {
                continue;
            }
            assert_eq!(
                macos_window.get(key),
                Some(value),
                "tauri.macos.conf.json must repeat `{key}` from tauri.conf.json unchanged — Tauri \
                 replaces the whole windows array, so a value left out here silently does not apply \
                 on macOS"
            );
        }

        for key in macos_window.keys() {
            assert!(
                base_window.contains_key(key) || MACOS_ONLY.contains(&key.as_str()),
                "tauri.macos.conf.json sets `{key}`, which is neither in the base config nor a \
                 known macOS-only key — add it to the base or to MACOS_ONLY deliberately"
            );
        }

        assert_eq!(
            macos_window.get("decorations"),
            Some(&serde_json::Value::Bool(true)),
            "macOS keeps its decorations: the native traffic lights and rounded corners come with them"
        );
        assert_eq!(
            base_window.get("decorations"),
            Some(&serde_json::Value::Bool(false)),
            "Windows and Linux draw our own controls, so the base config stays undecorated"
        );
    }
}
