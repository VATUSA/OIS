// Lets a data-backed widget body publish its fetch status up to the enclosing WidgetFrame, which
// renders a shared "last updated / refresh" control in the header. The frame provides the reporter
// via context; the widget body calls `useReportWidgetStatus(...)`. Presentational widgets (text,
// divider) never report, so their frames show no control.

import {createContext, useContext, useEffect, useRef} from "react";

export interface WidgetStatus {
  isFetching: boolean;
  /** ms epoch of the last successful load (React Query `dataUpdatedAt`), or 0 if never. */
  updatedAt: number;
  refetch: () => void;
}

const ReportContext = createContext<((s: WidgetStatus | null) => void) | null>(null);

export const WidgetStatusReporter = ReportContext.Provider;

/**
 * Publish this widget's data status to its frame. `refetch` may change identity every render (e.g.
 * multi-query sources); we stash it in a ref so the effect only re-fires on real status changes.
 */
export function useReportWidgetStatus(isFetching: boolean, updatedAt: number, refetch: () => void) {
  const report = useContext(ReportContext);
  const refetchRef = useRef(refetch);
  refetchRef.current = refetch;
  useEffect(() => {
    report?.({ isFetching, updatedAt, refetch: () => refetchRef.current() });
  }, [report, isFetching, updatedAt]);
  useEffect(() => () => report?.(null), [report]);
}
