import {useEffect, useState} from "react";

import {applyOrder, loadOrder, saveOrder} from "../lib/order-storage";

/**
 * Per-viewer custom ordering (see #109) — the order lives only in this browser. `ids` is the
 * current live set; returns the ordered list plus a setter that persists a full new order
 * (typically the result of `arrayMove` after a drag).
 */
export function usePersistedOrder(key: string, ids: string[]): [string[], (next: string[]) => void] {
  const [order, setOrder] = useState<string[]>(() => loadOrder(key));
  useEffect(() => saveOrder(key, order), [key, order]);
  return [applyOrder(ids, order), setOrder];
}
