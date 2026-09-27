//! Secondary windows belong to the main window: pop-outs (#349) and route windows (#350).
//!
//! Tauri keeps running until the *last* window closes. Without this, closing the main window left
//! the others running — an always-on-top panel floating over everything, a live process, no way
//! back to the main window — while notifications, which only the main window raises, silently
//! stopped (VATUSA/OIS#349 and #350 reviews). Closing the main window closes the app's layout.

use tauri::{Manager, Runtime, Window, WindowEvent};

/// The labels secondary windows carry (`web/src/lib/popout.ts`).
const SECONDARY_PREFIXES: [&str; 2] = ["popout-", "window-"];

/// Closes every secondary window once the main window is gone.
///
/// A destroy, not a close request: a route window's own close handler — which forgets it from the
/// remembered layout — never runs, so the layout comes back next launch.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if !main_window_gone(window.label(), event) {
        return;
    }
    for (label, secondary) in window.app_handle().webview_windows() {
        if is_secondary(&label) {
            let _ = secondary.destroy();
        }
    }
}

/// On `Destroyed` rather than `CloseRequested`, so that a close the app turns into a hide — as a
/// tray may — leaves the other windows where they are.
fn main_window_gone(label: &str, event: &WindowEvent) -> bool {
    label == "main" && matches!(event, WindowEvent::Destroyed)
}

fn is_secondary(label: &str) -> bool {
    SECONDARY_PREFIXES
        .iter()
        .any(|prefix| label.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_window_going_away_closes_the_others() {
        assert!(main_window_gone("main", &WindowEvent::Destroyed));
        // Anything short of the main window actually being destroyed leaves the others alone.
        assert!(!main_window_gone("main", &WindowEvent::Focused(false)));
        // A secondary window closing on its own takes nothing else with it.
        assert!(!main_window_gone("popout-fca-abc", &WindowEvent::Destroyed));
        assert!(!main_window_gone(
            "window--ops-idst-1x2y",
            &WindowEvent::Destroyed
        ));
    }

    #[test]
    fn closes_pop_outs_and_route_windows_and_nothing_else() {
        assert!(is_secondary("popout-fca-abc"));
        assert!(is_secondary("window--ops-idst-1x2y"));
        assert!(!is_secondary("main"));
        assert!(!is_secondary("settings"));
    }
}
