/** Which slice of `total` rows a DataTable shows, and which controls it offers. Pure, so it's tested. */
export type PageWindow = {
  start: number;
  end: number;
  /** The "Show all N" affordance (collapsed and there are more rows than the cap). */
  canExpand: boolean;
  /** The "Show fewer" affordance (expanded past the cap). */
  canCollapse: boolean;
  /** Client-side pagination controls. */
  pageCount: number;
  page: number;
};

export function pageWindow(
  total: number,
  opts: { rowCap: number; expanded: boolean; page: number; pageSize: number },
): PageWindow {
  const { rowCap, expanded, pageSize } = opts;
  const overCap = total > rowCap;
  if (overCap && !expanded) {
    return { start: 0, end: rowCap, canExpand: true, canCollapse: false, pageCount: 1, page: 1 };
  }
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const page = Math.min(Math.max(1, opts.page), pageCount);
  const start = (page - 1) * pageSize;
  return {
    start,
    end: Math.min(total, start + pageSize),
    canExpand: false,
    canCollapse: overCap,
    pageCount,
    page,
  };
}

/** Toggle `id` in a multi-selection, returning a new set. */
export function toggleId(selected: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(selected);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

/** Select-all over the visible ids: selects all when any is unselected, else clears them. */
export function toggleAll(selected: ReadonlySet<string>, ids: readonly string[]): Set<string> {
  const next = new Set(selected);
  const allOn = ids.length > 0 && ids.every((id) => next.has(id));
  for (const id of ids) {
    if (allOn) next.delete(id);
    else next.add(id);
  }
  return next;
}
