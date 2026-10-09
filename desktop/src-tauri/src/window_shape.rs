//! The main window's frame (#796).
//!
//! The main window uses each platform's standard title bar: macOS draws its own strip with the
//! traffic lights in their usual place, Windows draws minimize / maximize / close at the top right,
//! and Linux gets whatever its window manager draws. All of that is configuration in
//! `tauri.conf.json` (`decorations: true`), so there is no runtime code here; the tests below pin
//! that configuration, on every platform.

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

    /// Why a platform config must not touch `app.windows`, or `None` when it doesn't.
    fn reshapes_the_windows(config: &serde_json::Value) -> Option<&'static str> {
        config["app"].get("windows").map(|_| {
            "it overrides app.windows, which replaces the base config's decorated main window on \
             that platform (#796)"
        })
    }

    /// #796: macOS used to get its own shape from `tauri.macos.conf.json` (an `Overlay` title bar with
    /// the lights inset into the sidebar). Tauri merges a platform file over the base and **replaces**
    /// the whole `app.windows` array when it does, so a platform file that touches the windows at all
    /// decides that platform's frame on its own. None may: every platform takes the base config's
    /// decorated window. None of the files exists today, so the loop reads nothing; the next test
    /// proves the check fires on the override this branch deleted.
    #[test]
    fn no_platform_config_reshapes_the_windows() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in PLATFORM_CONFIGS {
            let text = match std::fs::read_to_string(dir.join(name)) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => panic!("{name}: {e}"),
            };
            let config: serde_json::Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            if let Some(why) = reshapes_the_windows(&config) {
                panic!("{name}: {why}");
            }
        }
    }

    /// The check above has to reject the macOS override #419 shipped, or it is decoration.
    #[test]
    fn the_old_macos_override_would_be_rejected() {
        let old = serde_json::json!({
            "app": {"windows": [{
                "label": "main",
                "decorations": true,
                "titleBarStyle": "Overlay",
                "hiddenTitle": true,
                "trafficLightPosition": {"x": 10, "y": 28}
            }]}
        });
        assert!(reshapes_the_windows(&old).is_some());

        let unrelated = serde_json::json!({"bundle": {"macOS": {"minimumSystemVersion": "11.0"}}});
        assert!(reshapes_the_windows(&unrelated).is_none());
    }
}
