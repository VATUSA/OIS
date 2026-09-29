import {useDragRegionProps, WindowControls} from "./window-controls";

/**
 * The frameless main window's controls for the layouts that render *outside* `AppShell` (#423).
 *
 * `AppShell` puts `WindowControls` in `Shell`'s `leading` slot, but two layouts return from
 * `RootLayout` before reaching it — the backend-unreachable error screen and the signed-out landing
 * page. `decorations: false` means those had no native title bar *and* no controls: an undecorated
 * rectangle with no in-app way to close it, on the first screen a new desktop user sees.
 *
 * Mounted in those two layouts rather than above the layout branch, because above it this would draw
 * a second set of controls on top of the shell's own for every signed-in page.
 *
 * `useDragRegionProps()` is deliberately the only gate here: it is already `undefined` on the web
 * build and in route windows (#350), which keep their native title bar. Reusing that one answer
 * instead of a platform check of our own is what keeps this correct when the gate changes — #419
 * gives macOS its decorations back, and this follows without an edit.
 */
export function WindowChromeBar() {
  const dragRegion = useDragRegionProps();
  if (!dragRegion) return null;

  // Left, matching `Shell`'s `leading` slot, so the signed-out window reads like the signed-in one.
  // Nothing else in these layouts is positioned or stacked, and both start below this strip — the
  // landing hero opens on padding, the error screen is vertically centred — so it covers no content.
  return (
    <div {...dragRegion} className="fixed inset-x-0 top-0 z-50 flex h-9 items-center px-2">
      <WindowControls />
    </div>
  );
}
