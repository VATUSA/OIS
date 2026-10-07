//! The main window's frame (#796).
//!
//! The main window uses each platform's standard title bar: macOS draws its own strip with the
//! traffic lights in their usual place, Windows draws minimize / maximize / close at the top right,
//! and Linux gets whatever its window manager draws. All of that is configuration in
//! `tauri.conf.json` (`decorations: true`), so there is no runtime code here.
//!
//! This reverses #402 and #419, which hid the title bar and had the app draw its own controls: a
//! replica of the macOS lights on Windows and Linux, the real lights inset into the sidebar on macOS
//! through a `tauri.macos.conf.json` override, and a DWM call to round the undecorated Windows
//! window. A decorated window gets its corners, shadow, snap layouts and double-click-to-maximize
//! from the OS, so all of that is gone. What is left is the test below, which keeps it gone.

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// The window-frame keys that only matter when the app hides or overlays the native title bar.
    /// Any of them on the main window brings back a shape the app then has to draw chrome for.
    const FRAMELESS_KEYS: [&str; 4] = [
        "titleBarStyle",
        "hiddenTitle",
        "trafficLightPosition",
        "transparent",
    ];

    /// The per-platform overrides Tauri merges over `tauri.conf.json` when it builds on that OS.
    const PLATFORM_CONFIGS: [&str; 3] = [
        "tauri.macos.conf.json",
        "tauri.windows.conf.json",
        "tauri.linux.conf.json",
    ];

    fn main_window(config: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
        config["app"]["windows"]
            .as_array()
            .expect("app.windows is an array")
            .iter()
            .find(|w| w["label"] == "main")
            .and_then(serde_json::Value::as_object)
            .expect("app.windows has a `main` window")
    }

    /// #796: the base config is the one every platform builds from, and its main window is decorated
    /// with no frameless styling, so Windows and Linux get their native title bar.
    #[test]
    fn the_main_window_keeps_the_native_title_bar() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let window = main_window(&base);

        assert_eq!(
            window.get("decorations"),
            Some(&serde_json::Value::Bool(true)),
            "the main window must keep the OS's own title bar and window buttons (#796)"
        );
        for key in FRAMELESS_KEYS {
            assert!(
                !window.contains_key(key),
                "tauri.conf.json sets `{key}` on the main window, which hides or overlays the native \
                 title bar (#796)"
            );
        }
    }

    /// #796: macOS used to get its own shape from `tauri.macos.conf.json` (an `Overlay` title bar with
    /// the lights inset into the sidebar). Tauri merges a platform file over the base and **replaces**
    /// the whole `app.windows` array when it does, so a platform file that touches the windows at all
    /// decides that platform's frame on its own. None may: every platform takes the base config's
    /// decorated window.
    #[test]
    fn no_platform_config_reshapes_the_windows() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in PLATFORM_CONFIGS {
            let path = dir.join(name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let config: serde_json::Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                config["app"].get("windows").is_none(),
                "{name} overrides app.windows, which replaces the base config's decorated main window \
                 on that platform (#796)"
            );
        }
    }
}
