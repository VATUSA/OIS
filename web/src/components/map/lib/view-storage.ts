/**
 * Per-map-instance camera persistence in localStorage. The key namespaces each map (`ops-fca`,
 * `advisories`, `dash-<widgetId>`), so a dashboard's map widgets each remember their own view. This is
 * view state, not a synced setting — it always lives in localStorage (the `map.persistView` setting
 * only gates whether we read/write it).
 */

export interface StoredView {
  longitude: number;
  latitude: number;
  zoom: number;
  pitch?: number;
  bearing?: number;
}

const storageKey = (key: string) => `ois.mapview.${key}`;

const num = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

/** Read a saved view for `key`, or null if absent/invalid/unavailable. */
export function loadView(key: string): StoredView | null {
  try {
    const raw = localStorage.getItem(storageKey(key));
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<StoredView>;
    if (num(v.longitude) && num(v.latitude) && num(v.zoom)) {
      return {
        longitude: v.longitude,
        latitude: v.latitude,
        zoom: v.zoom,
        pitch: num(v.pitch) ? v.pitch : undefined,
        bearing: num(v.bearing) ? v.bearing : undefined,
      };
    }
  } catch {
    /* private mode / disabled / bad JSON — non-fatal */
  }
  return null;
}

/** Persist a view for `key` (best-effort). */
export function saveView(key: string, v: StoredView): void {
  try {
    localStorage.setItem(storageKey(key), JSON.stringify(v));
  } catch {
    /* non-fatal */
  }
}
