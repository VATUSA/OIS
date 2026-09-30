import {useDragRegionProps, WindowChromeSlot} from "./window-controls";

/**
 * The main window's own chrome for the layouts that render *outside* `AppShell` (#423).
 *
 * `AppShell` puts the window's buttons at the leading edge of the sidebar's chrome row, but two
 * layouts return from `RootLayout` before reaching it — the backend-unreachable error screen and the
 * signed-out landing page. On Windows and Linux `decorations: false` means those had no native title
 * bar *and* no buttons: an undecorated rectangle with no in-app way to close it, on the first screen
 * a new desktop user sees.
 *
 * Mounted in those two layouts rather than above the layout branch, because above it this would draw
 * a second set on top of the shell's own for every signed-in page.
 *
 * Two gates, not one, because they answer different questions (#419):
 *
 * - {@link useDragRegionProps} decides whether this strip exists at all. It is already `undefined` on
 *   the web build and in route windows (#350), which keep their native title bar, so reusing that one
 *   answer avoids a platform check of our own. On macOS it is `true` — an `Overlay` title bar is
 *   transparent and needs the drag region as much as an undecorated window does — so the strip is
 *   drawn there too, and makes the top of the signed-out window draggable.
 * - {@link WindowChromeSlot} decides what goes *in* it. On macOS that is nothing, because the OS
 *   paints the real traffic lights over this spot itself; elsewhere it is the replica. It also owns
 *   how wide the buttons are, which differs per platform and must not be guessed here.
 *
 * The geometry matches the shell's chrome row — `px-2.5` and a 44px strip, the same `h-11` as
 * `Shell`'s top bar — so the buttons do not move when the user signs in or out (#423 review). That
 * matters on Windows and Linux, where the replica is ours to place; on macOS the OS holds the lights
 * at fixed window coordinates regardless.
 *
 * The strip covers no content: the landing hero opens on 72px of padding and the error screen is
 * vertically centred, and nothing else in either layout is positioned or stacked.
 */
export function WindowChromeBar() {
  const dragRegion = useDragRegionProps();
  if (!dragRegion) return null;

  return (
    <div {...dragRegion} className="fixed inset-x-0 top-0 z-50 flex h-11 items-center px-2.5">
      <WindowChromeSlot />
    </div>
  );
}
