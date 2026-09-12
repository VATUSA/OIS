/**
 * Per-viewer custom ordering for a list of ids, in localStorage (see #109). Never synced — a
 * personal declutter/preference, not shared state.
 */

const storageKey = (key: string) => `ois.order.${key}`;

/** Read a saved order for `key`, or [] if absent/invalid/unavailable. */
export function loadOrder(key: string): string[] {
  try {
    const raw = localStorage.getItem(storageKey(key));
    if (!raw) return [];
    const v: unknown = JSON.parse(raw);
    return Array.isArray(v) && v.every((x) => typeof x === "string") ? v : [];
  } catch {
    return [];
  }
}

/** Persist an order for `key` (best-effort). */
export function saveOrder(key: string, order: string[]): void {
  try {
    localStorage.setItem(storageKey(key), JSON.stringify(order));
  } catch {
    /* private mode / disabled / quota exceeded — non-fatal */
  }
}

/**
 * Apply a stored order to the current live `ids`: items present in `order` come first, in that
 * relative order; any id not yet ordered (new items, or the very first run) is appended at the end
 * in its natural (incoming) order, so a freshly-created item shows up without needing to be
 * dragged into place first.
 */
export function applyOrder(ids: string[], order: string[]): string[] {
  const known = new Set(ids);
  const kept = order.filter((id) => known.has(id));
  const keptSet = new Set(kept);
  const missing = ids.filter((id) => !keptSet.has(id));
  return [...kept, ...missing];
}
