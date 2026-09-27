/**
 * Which route windows were open, so the same set comes back next launch (#350).
 *
 * Only *which* — where each one sits is already remembered per window by `lib/popout.ts`. Both live
 * in `localStorage` for the same reason: a layout spread across three monitors at a facility is not
 * the layout you want on a laptop, so it belongs to the machine rather than to the account.
 */

export type RememberedWindow = {
  /** Stable id, derived from the route, so reopening raises rather than duplicates. */
  id: string;
  route: string;
  title: string;
};

const STORAGE_KEY = "ois.windows";

/** The remembered set, or empty when there is nothing usable stored. */
export function rememberedWindows(): RememberedWindow[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];

    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];

    // Drop anything malformed rather than failing the whole restore: one bad entry written by an
    // older version shouldn't cost the user every other window.
    return parsed.filter(
      (w): w is RememberedWindow =>
        !!w &&
        typeof w === "object" &&
        typeof (w as RememberedWindow).id === "string" &&
        typeof (w as RememberedWindow).route === "string" &&
        typeof (w as RememberedWindow).title === "string",
    );
  } catch {
    return [];
  }
}

function write(windows: RememberedWindow[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(windows));
  } catch {
    // Not remembering the layout is a lesser failure than refusing to open the window.
  }
}

/** Records a window as open. Re-recording the same id replaces it rather than duplicating. */
export function rememberWindow(win: RememberedWindow) {
  write([...rememberedWindows().filter((w) => w.id !== win.id), win]);
}

/**
 * Forgets a window.
 *
 * Called when the user closes one — closing a window is how you say "I don't want this next time",
 * so it has to stick.
 */
export function forgetWindow(id: string) {
  write(rememberedWindows().filter((w) => w.id !== id));
}
