//! Pop-out mini-windows belong to the main window (#349).
//!
//! Tauri keeps running until the *last* window closes, so without this, closing the main window
//! with a pop-out open left an always-on-top panel floating over everything, a live process, and no
//! way back to the main window — while notifications, which only the main window raises, silently
//! stopped (VATUSA/OIS#349 review).

use tauri::{Manager, Runtime, Window, WindowEvent};

/// The label every pop-out window carries (`web/src/lib/popout.ts`).
const POPOUT_PREFIX: &str = "popout-";

/// Closes every pop-out once the main window is gone.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if !main_window_gone(window.label(), event) {
        return;
    }
    for (label, popout) in window.app_handle().webview_windows() {
        if is_popout(&label) {
            let _ = popout.destroy();
        }
    }
}

/// On `Destroyed` rather than `CloseRequested`, so that a close the app turns into a hide — as a
/// tray may — leaves the pop-outs where they are.
fn main_window_gone(label: &str, event: &WindowEvent) -> bool {
    label == "main" && matches!(event, WindowEvent::Destroyed)
}

fn is_popout(label: &str) -> bool {
    label.starts_with(POPOUT_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_window_going_away_closes_pop_outs() {
        assert!(main_window_gone("main", &WindowEvent::Destroyed));
        // Anything short of the main window actually being destroyed leaves the pop-outs alone.
        assert!(!main_window_gone("main", &WindowEvent::Focused(false)));
        // A pop-out closing on its own takes nothing else with it.
        assert!(!main_window_gone("popout-fca-abc", &WindowEvent::Destroyed));
    }

    #[test]
    fn closes_pop_outs_and_nothing_else() {
        assert!(is_popout("popout-fca-abc"));
        assert!(is_popout("popout-widget-1-3k9x"));
        assert!(!is_popout("main"));
        assert!(!is_popout("settings"));
    }
}
