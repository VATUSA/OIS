import {keepPreviousData, useQuery} from "@tanstack/react-query";

import type {components} from "@ois/api-client";

import {ois} from "./api";

export type AuditFilters = { q?: string; from?: string; to?: string };

export function useAuditLog(page = 1, pageSize = 50, filters: AuditFilters = {}, { enabled = true } = {}) {
  const q = filters.q?.trim() || undefined;
  const from = filters.from || undefined;
  const to = filters.to || undefined;
  return useQuery({
    queryKey: ["audit", page, pageSize, q, from, to],
    enabled,
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/admin/audit", {
        params: { query: { page, page_size: pageSize, q, from, to } },
      });
      if (error || !data) throw new Error("failed to load audit log");
      return data;
    },
    // Keep the current page on screen while the next one loads (no flash to "Loading…").
    placeholderData: keepPreviousData,
  });
}

export function useFacilities() {
  return useQuery({
    queryKey: ["facilities"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/facilities");
      if (error || !data) throw new Error("failed to load facilities");
      return data;
    },
  });
}

export type AdminSummary = components["schemas"]["AdminSummaryBody"];
export type DailySeries = components["schemas"]["DailySeries"];

/** The Admin landing summary; each section is null when the user lacks that page's permission. */
export function useAdminSummary() {
  return useQuery({
    queryKey: ["admin-summary"],
    queryFn: async (): Promise<AdminSummary> => {
      const { data, error } = await ois.GET("/api/v1/admin/summary");
      if (error || !data) throw new Error("failed to load admin summary");
      return data;
    },
    refetchInterval: 60_000,
  });
}

/** Last `window` days vs the `window` days before them, as a direction + "12.5% (+3)" text. */
export function weekOverWeek(series: DailySeries, window = 7): { direction: "up" | "down" | "flat"; text: string } {
  const counts = series.points.map((p) => p.count);
  const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0);
  const recent = sum(counts.slice(-window));
  const prior = sum(counts.slice(-2 * window, -window));
  const delta = recent - prior;
  const direction = delta > 0 ? "up" : delta < 0 ? "down" : "flat";
  const pct = prior === 0 ? (recent === 0 ? 0 : 100) : Math.abs((delta / prior) * 100);
  return { direction, text: `${pct.toFixed(1)}% (${delta >= 0 ? "+" : ""}${delta})` };
}
