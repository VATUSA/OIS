import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import {useToast} from "@ois/ui";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type MonitorTable = components["schemas"]["MonitorTableBody"];
export type MonitorRow = components["schemas"]["MonitorRowBody"];
export type MonitorBin = components["schemas"]["MonitorBinBody"];

/** Time Range stops, in hours. The server always computes six; this only slices what is drawn. */
export const TIME_RANGES = [2, 3, 4, 5, 6] as const;
export const DEFAULT_TIME_RANGE = 4;
/** "Show if alerted in next N hours" choices; `0` shows every sector. */
export const ALERT_WINDOWS = [0, 0.5, 1, 1.5, 2, 3, 4, 6] as const;
export const DEFAULT_ALERT_WINDOW = 2;
const BIN_MIN = 15;
const HOUR_MS = 3_600_000;

export const monitorKey = (artcc: string) => ["monitor", artcc] as const;

/** An ARTCC's Monitor (#701), refreshed every minute. Six hours of bins whatever the Time Range. */
export function useMonitorTable(artcc: string) {
  return useQuery({
    queryKey: monitorKey(artcc),
    enabled: artcc !== "",
    refetchInterval: 60_000,
    queryFn: async (): Promise<MonitorTable> => {
      const { data, error } = await ois.GET("/api/v1/flow/monitor/{artcc}", {
        params: { path: { artcc } },
      });
      if (error || !data) throw new Error("failed to load the Monitor");
      return data;
    },
  });
}

export const monitorNeighboursKey = (artcc: string) => ["monitor-neighbours", artcc] as const;

/** An ARTCC's first-tier neighbours (#712), whose tables follow its own, view-only. They rarely change. */
export function useMonitorNeighbours(artcc: string) {
  return useQuery({
    queryKey: monitorNeighboursKey(artcc),
    enabled: artcc !== "",
    staleTime: 60 * 60_000,
    queryFn: async (): Promise<string[]> => {
      const { data, error } = await ois.GET("/api/v1/flow/monitor/{artcc}/neighbours", {
        params: { path: { artcc } },
      });
      if (error || !data) throw new Error("failed to load the neighbours");
      return data.neighbours;
    },
  });
}

/** Work `sectorId` at `target`'s position (#599). */
export function useConsolidate(artcc: string) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ sectorId, target }: { sectorId: string; target: string }) => {
      const { response } = await ois.PUT("/api/v1/flow/monitor/{artcc}/consolidations/{sector_id}", {
        params: { path: { artcc, sector_id: sectorId } },
        body: { target_sector_id: target },
      });
      if (!response.ok) throw new Error("consolidate failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: monitorKey(artcc) }),
    onError: () => toast.error("Couldn’t consolidate the sector"),
  });
}

/** Give `sectorId` back its own row. */
export function useReleaseSector(artcc: string) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (sectorId: string) => {
      const { response } = await ois.DELETE("/api/v1/flow/monitor/{artcc}/consolidations/{sector_id}", {
        params: { path: { artcc, sector_id: sectorId } },
      });
      if (!response.ok) throw new Error("release failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: monitorKey(artcc) }),
    onError: () => toast.error("Couldn’t release the sector"),
  });
}

/** The bins the Time Range shows. A pure slice of the six hours already loaded: no refetch. */
export function sliceBins(bins: MonitorBin[], hours: number): MonitorBin[] {
  return bins.slice(0, Math.round((hours * 60) / BIN_MIN));
}

/**
 * Whether `row` has an amber or red bin starting within the next `hours` (the bin containing now
 * counts). Judged on the full six hours, so it is independent of the Time Range: a 4-hour table can
 * filter on the next 1.5 hours. `hours` of 0 shows every row.
 */
export function alertedWithin(row: MonitorRow, hours: number, nowMs: number): boolean {
  if (hours <= 0) return true;
  const until = nowMs + hours * HOUR_MS;
  return row.bins.some((b) => b.alert !== "green" && Date.parse(b.start) < until);
}

/** The centre's sector order: numeric when both ids are numbers, so 16 comes before 100. */
export function defaultOrder(ids: string[]): string[] {
  const numeric = (s: string) => /^\d+$/.test(s);
  return [...ids].sort((a, b) =>
    numeric(a) && numeric(b) ? Number(a) - Number(b) : a.localeCompare(b),
  );
}

/**
 * Move `id` one place up (`-1`) or down (`1`) among the rows shown: it steps past the next *visible*
 * row, so it still does the obvious thing while the alert filter is hiding rows. A move off either
 * end, or of a row that isn't shown, changes nothing.
 */
export function moveRow(order: string[], id: string, dir: 1 | -1, visible: ReadonlySet<string>): string[] {
  const from = order.indexOf(id);
  if (from < 0 || !visible.has(id)) return order;
  let to = from + dir;
  while (to >= 0 && to < order.length && !visible.has(order[to])) to += dir;
  if (to < 0 || to >= order.length) return order;
  const next = order.filter((x) => x !== id);
  const anchor = next.indexOf(order[to]);
  next.splice(dir === 1 ? anchor + 1 : anchor, 0, id);
  return next;
}
