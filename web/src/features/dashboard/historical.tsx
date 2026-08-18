// Historical dashboard mode. When a board is rendered inside a `HistoricalProvider`, its
// data-source widgets recompute at the scrubber instant `t` instead of live. A null context (the
// default everywhere else) means live — so the ops dashboard is untouched.

import {createContext, type ReactNode, useContext} from "react";

export interface HistoricalWindow {
  /** Window bounds + scrubber instant, all Unix epoch seconds. */
  from: number;
  to: number;
  t: number;
}

const HistoricalContext = createContext<HistoricalWindow | null>(null);

export function HistoricalProvider({
  value,
  children,
}: {
  value: HistoricalWindow;
  children: ReactNode;
}) {
  return <HistoricalContext.Provider value={value}>{children}</HistoricalContext.Provider>;
}

/** The active historical window, or null when live. */
export function useHistorical(): HistoricalWindow | null {
  return useContext(HistoricalContext);
}

/** The scrubber instant (Unix seconds) to reconstruct at, or null when live. */
export function useHistoricalAt(): number | null {
  return useContext(HistoricalContext)?.t ?? null;
}
