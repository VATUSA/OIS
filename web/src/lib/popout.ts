import {can} from "@/lib/platform";
import {clampToMonitors, toLogical, type Monitor, type Rect} from "@/lib/popout-geometry";

/**
 * Pop-out mini-windows (#349) — small always-on-top native windows holding one panel, so a
 * controller can keep a metering ladder or a widget over CRC, vATIS or charts.
 *
 * A pop-out is just the app at a bare route: `?embed=1` (`web/src/router.tsx`) already strips the
 * shell, and a *relative* URL resolves against the dev server in development and the asset protocol
 * in production, so there is no environment branching here.
 *
 * It does not share the main window's react-query cache — each Tauri window is its own webview, so
 * it can't. It doesn't need to: the pop-out mounts the same providers and the same realtime socket,
 * and the server is the source of truth, so both windows move in step on the same nudges.
 *
 * This is also the window plumbing multi-window (#350) builds on, which is why opening is keyed on
 * a caller-supplied id rather than hardcoded to panels.
 */

export type PopoutSpec = {
  /** Stable per-panel id. Becomes the window label, so reopening the same panel raises it. */
  id: string;
  title: string;
  /** App-relative route, e.g. `/popout/fca/abc`. `?embed=1` is added here. */
  route: string;
  width?: number;
  height?: number;
};

const DEFAULT_SIZE = {width: 380, height: 520};
const LABEL_PREFIX = "popout-";

/** How long a drag or resize must be still before the new geometry is written. */
const GEOMETRY_SETTLE_MS = 300;

/**
 * Window labels must be simple; panel ids can be UUIDs or paths, so normalise.
 *
 * A plain character substitution is not enough on its own: `ZDC_ARR` and `ZDC.ARR` both flatten to
 * `ZDC-ARR`, and two different panels sharing one label means opening the second raises the first.
 * Anything that had to be rewritten therefore carries a short hash of the original id, so distinct
 * ids stay distinct while the label stays readable for the common case.
 */
export function popoutLabel(id: string): string {
  const flattened = id.replace(/[^a-zA-Z0-9-]/g, "-");
  if (flattened === id) return `${LABEL_PREFIX}${id}`;

  // djb2 — a label disambiguator, not a security boundary.
  let hash = 5381;
  for (let i = 0; i < id.length; i += 1) hash = ((hash << 5) + hash + id.charCodeAt(i)) >>> 0;
  return `${LABEL_PREFIX}${flattened}-${hash.toString(36)}`;
}

function storageKey(id: string): string {
  return `ois.popout.${id}`;
}

/** Geometry is per machine, not per user: a position that suits a three-monitor desk is wrong on a laptop. */
function readGeometry(id: string): Rect | undefined {
  try {
    const raw = localStorage.getItem(storageKey(id));
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as Partial<Rect>;
    const {x, y, width, height} = parsed;
    if ([x, y, width, height].some((n) => typeof n !== "number" || !Number.isFinite(n))) {
      return undefined;
    }
    return {x: x!, y: y!, width: width!, height: height!};
  } catch {
    // Private mode, cleared site data, or something else wrote nonsense here. Let the OS place it.
    return undefined;
  }
}

function writeGeometry(id: string, rect: Rect) {
  try {
    localStorage.setItem(storageKey(id), JSON.stringify(rect));
  } catch {
    // Not being able to remember where a window was is not worth surfacing.
  }
}

/**
 * Opens a panel in its own always-on-top window, or raises it if it is already open.
 *
 * Raising rather than duplicating matters: clicking "pop out" twice should not leave two identical
 * windows fighting for the same screen space.
 *
 * A no-op on the web build, where `can("miniWindows")` is false.
 */
export async function openPopout(spec: PopoutSpec): Promise<boolean> {
  if (!can("miniWindows")) return false;

  const label = popoutLabel(spec.id);

  try {
    const {WebviewWindow} = await import("@tauri-apps/api/webviewWindow");
    const {availableMonitors} = await import("@tauri-apps/api/window");

    const existing = await WebviewWindow.getByLabel(label);
    if (existing) {
      await existing.unminimize().catch(() => undefined);
      await existing.show();
      await existing.setFocus();
      return true;
    }

    const monitors = (await availableMonitors()) as unknown as Monitor[];
    const restored = clampToMonitors(readGeometry(spec.id), monitors);
    const size = {
      width: restored?.width ?? spec.width ?? DEFAULT_SIZE.width,
      height: restored?.height ?? spec.height ?? DEFAULT_SIZE.height,
    };

    const separator = spec.route.includes("?") ? "&" : "?";
    const win = new WebviewWindow(label, {
      url: `${spec.route}${separator}embed=1`,
      title: spec.title,
      width: size.width,
      height: size.height,
      ...(restored ? {x: restored.x, y: restored.y} : {}),
      alwaysOnTop: true,
      resizable: true,
      // Big enough that the panels we allow out stay legible; they declare minimums around 200-350px.
      minWidth: 260,
      minHeight: 200,
    });

    // Remember where the user put it. Both events are needed — a move and a resize are separate —
    // and both fire continuously while dragging, so the write is debounced: otherwise a single
    // drag across the screen would mean hundreds of synchronous localStorage writes.
    let settle: number | undefined;
    const remember = () => {
      window.clearTimeout(settle);
      settle = window.setTimeout(async () => {
        try {
          // Tauri reports these in physical pixels; stored geometry is logical (see `Rect`).
          const [position, outer, factor] = await Promise.all([
            win.outerPosition(),
            win.outerSize(),
            win.scaleFactor(),
          ]);
          writeGeometry(
            spec.id,
            toLogical({x: position.x, y: position.y, width: outer.width, height: outer.height}, factor),
          );
        } catch {
          // The window is probably closing; nothing to remember.
        }
      }, GEOMETRY_SETTLE_MS);
    };

    await win.onMoved(remember);
    await win.onResized(remember);

    return true;
  } catch {
    return false;
  }
}

/** Closes a pop-out if it's open. Closing one that isn't is not an error. */
export async function closePopout(id: string): Promise<void> {
  if (!can("miniWindows")) return;

  try {
    const {WebviewWindow} = await import("@tauri-apps/api/webviewWindow");
    const win = await WebviewWindow.getByLabel(popoutLabel(id));
    await win?.close();
  } catch {
    // Already gone.
  }
}
