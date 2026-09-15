/**
 * TMU tab resolution + last-visited-tab persistence (see #247). `resolveTmuTab` is the pure part
 * (unit-tested); the localStorage helpers are best-effort, matching
 * `web/src/components/map/lib/order-storage.ts`'s pattern.
 */

export const TAB_IDS = [
  "programs",
  "restrictions",
  "ground-stops",
  "gdp",
  "rate-calculator",
] as const;

export type Tab = (typeof TAB_IDS)[number];

const LAST_TAB_KEY = "ois.tmu.lastTab";

/**
 * Given the tabs the current user can see and the tab the URL requested, pick the active one and
 * say whether the URL needs to be rewritten to match. `needsUrlSync` is true only when a tab WAS
 * requested but isn't visible to this user (a permission-fallback) — never for a bare `/ops/tmu`
 * with no `tab` param, which should stay bare.
 */
export function resolveTmuTab(
  tabs: { id: Tab }[],
  requestedTab: Tab | undefined,
): { active: Tab | undefined; needsUrlSync: boolean } {
  const visible = tabs.some((t) => t.id === requestedTab);
  const active = visible ? requestedTab : tabs[0]?.id;
  return { active, needsUrlSync: requestedTab != null && !visible };
}

/**
 * Whether a `resolveTmuTab` result represents a tab the user actually landed on by choice — a
 * bare `/ops/tmu`, or a validly requested tab — as opposed to an involuntary permission-fallback
 * redirect (`needsUrlSync`). Only the former should overwrite the saved "last viewed" tab; saving
 * on a fallback would clobber the user's real preference with whatever tab they got bounced to
 * (#247 rework — `saveLastTmuTab` used to fire on every `active` change, fallback included).
 */
export function shouldSaveLastTmuTab(resolved: {
  active: Tab | undefined;
  needsUrlSync: boolean;
}): resolved is { active: Tab; needsUrlSync: false } {
  return resolved.active != null && !resolved.needsUrlSync;
}

/** The last tab the user actively viewed, or undefined if unset/unavailable (private mode etc). */
export function loadLastTmuTab(): Tab | undefined {
  try {
    const v = localStorage.getItem(LAST_TAB_KEY);
    return TAB_IDS.includes(v as Tab) ? (v as Tab) : undefined;
  } catch {
    return undefined;
  }
}

/** Persist the last-viewed tab (best-effort). */
export function saveLastTmuTab(tab: Tab): void {
  try {
    localStorage.setItem(LAST_TAB_KEY, tab);
  } catch {
    /* private mode / disabled / quota exceeded — non-fatal */
  }
}
