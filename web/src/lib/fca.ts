import {keepPreviousData, useMutation, useQueries, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";
import {useHistoricalAt} from "./historical-context";
import {fetchHistTraffic} from "./historical";

export type Fca = components["schemas"]["FcaBody"];
export type UpsertFca = components["schemas"]["UpsertFcaRequest"];
export type TrafficAircraft = components["schemas"]["TrafficAircraft"];
export type AtcBoard = components["schemas"]["AtcBoard"];
export type AtcAirport = components["schemas"]["AtcAirport"];
export type AtcArea = components["schemas"]["AtcArea"];
export type AtcCenter = components["schemas"]["AtcCenter"];
export type AtcPosition = components["schemas"]["AtcPosition"];
export type FcaFlight = components["schemas"]["FcaFlight"];
export type FixValidation = components["schemas"]["FixValidationBody"];

/** A metered delay of ~1 min or more is worth flagging (below that is rounding noise). */
export const DELAY_THRESHOLD_SEC = 30;

/** Delay as `M:SS` (e.g. 1268 → "21:08"). */
export function fmtDelaySec(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/**
 * Which of the given route-fix tokens aren't real nav fixes (typos that would silently exclude
 * traffic). Debounce `fixes` before passing it in. Needs `flow.fca.read`.
 */
export function useValidateFixes(fixes: string) {
  const trimmed = fixes.trim();
  return useQuery({
    queryKey: ["validate-fixes", trimmed],
    enabled: trimmed.length > 0,
    queryFn: async (): Promise<FixValidation> => {
      const { data, error } = await ois.GET("/api/v1/flow/validate-fixes", {
        params: { query: { fixes: trimmed } },
      });
      if (error || !data) throw new Error("validate failed");
      return data;
    },
    staleTime: 60_000,
    retry: false,
  });
}
export type AircraftRoute = components["schemas"]["AircraftRoute"];
export type DataStatus = components["schemas"]["DataStatus"];
export type CoverageReport = components["schemas"]["CoverageReport"];

/** How much of live filed traffic the nav engine resolves; refreshed every 60s. */
export function useRouteCoverage() {
  return useQuery({
    queryKey: ["route-coverage"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/route-coverage");
      if (error || !data) throw new Error("failed to load coverage");
      return data;
    },
    staleTime: 60_000,
    refetchInterval: 60_000,
    retry: false,
  });
}

/** Age in days of a `YYYY-MM-DD` NASR cycle, or `null` if it doesn't parse. */
export function cycleAgeDays(cycle: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(cycle);
  if (!m) return null;
  const d = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return Math.floor((Date.now() - d) / 86_400_000);
}

/** Health of the runtime nav + winds data, refreshed every 60s. */
export function useDataStatus() {
  return useQuery({
    queryKey: ["flow-data-status"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/data-status");
      if (error || !data) throw new Error("failed to load data status");
      return data;
    },
    staleTime: 60_000,
    refetchInterval: 60_000,
  });
}

/**
 * The toast a completed data refresh should raise. The endpoint answers 200 even when the nav fetch
 * could only fall back to an older cycle — it keeps last-good rather than failing — so a stale or
 * unreadable cycle must not read as success (VATUSA/OIS#317). Pressed from the stale banner (#332),
 * this is the only evidence of what the refresh actually did.
 */
export function refreshToast(data: DataStatus): {
  variant: "success" | "warning";
  title: string;
  description?: string;
} {
  const summary = `Nav ${data.nav_cycle} · ${data.winds_stations} wind stations`;
  const behind = data.nav_cycles_behind;
  if (behind == null)
    return {
      variant: "warning",
      title: `NASR cycle ${data.nav_cycle} is unreadable`,
      description: summary,
    };
  if (behind > 0)
    return {
      variant: "warning",
      title: `NASR cycle ${data.nav_cycle} is ${behind} cycle${behind === 1 ? "" : "s"} behind`,
      description: `${summary} · current ${data.nav_cycle_current}`,
    };
  return { variant: "success", title: summary };
}

/** Force an immediate nav + winds refresh (requires flow.fca.update). */
export function useRefreshData() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async () => {
      const { data, error, response } = await ois.POST("/api/v1/flow/data-refresh");
      if (response.status === 409) throw new Error("already");
      if (error || !data) throw new Error("failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["flow-data-status"], data);
      // Freshly resolved routes may shift matches/ETAs.
      queryClient.invalidateQueries({ queryKey: ["fca-traffic"] });
      queryClient.invalidateQueries({ queryKey: ["aircraft-route"] });
      const { variant, title, description } = refreshToast(data);
      toast[variant](title, description ? { description } : undefined);
    },
    onError: (e) =>
      toast.error(
        e instanceof Error && e.message === "already"
          ? "A refresh is already running — give it a moment."
          : "Refresh failed",
      ),
  });
}

/** All FCAs (shared across controllers). */
/** React Query key for an FCA list — the shared set, or one event's planned/published/archived set. */
const fcaKey = (eventId?: number) => (eventId == null ? ["fcas"] : ["event-fcas", eventId]);

/** FCAs. With `eventId` this is the event manager's builder set (planned + published + archived,
 *  from `/events/{id}/fcas`); otherwise the shared live set (or a historical snapshot). */
export function useFcas(eventId?: number) {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: eventId != null ? fcaKey(eventId) : at == null ? ["fcas"] : ["hist-fcas", at],
    queryFn: async () => {
      if (eventId != null) {
        const { data, error } = await ois.GET("/api/v1/events/{id}/fcas", {
          params: { path: { id: eventId } },
        });
        if (error || !data) throw new Error("failed to load event FCAs");
        return data;
      }
      if (at != null) {
        const { data, error } = await ois.GET("/api/v1/stats/hist/fcas", {
          params: { query: { at } },
        });
        if (error || !data) throw new Error("failed to load historical FCAs");
        return data;
      }
      const { data, error } = await ois.GET("/api/v1/flow/fcas");
      if (error || !data) throw new Error("failed to load FCAs");
      return data;
    },
    staleTime: eventId != null || at == null ? undefined : Infinity,
    placeholderData: eventId == null && at != null ? keepPreviousData : undefined,
  });
}

export function useCreateFca(eventId?: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertFca) => {
      if (eventId != null) {
        const { data, error } = await ois.POST("/api/v1/events/{id}/fcas", {
          params: { path: { id: eventId } },
          body,
        });
        if (error || !data) throw new Error("create failed");
        return data;
      }
      const { data, error } = await ois.POST("/api/v1/flow/fcas", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: fcaKey(eventId) });
      toast.success("FCA created");
    },
    onError: () => toast.error("Couldn’t create the FCA"),
  });
}

export function useUpdateFca(eventId?: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertFca }) => {
      if (eventId != null) {
        const { data, error } = await ois.PUT("/api/v1/events/{id}/fcas/{fca_id}", {
          params: { path: { id: eventId, fca_id: id } },
          body,
        });
        if (error || !data) throw new Error("save failed");
        return data;
      }
      const { data, error } = await ois.PUT("/api/v1/flow/fcas/{id}", {
        params: { path: { id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: fcaKey(eventId) });
    },
    onError: () => toast.error("Couldn’t save the FCA"),
  });
}

export function useDeleteFca(eventId?: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      if (eventId != null) {
        const { error } = await ois.DELETE("/api/v1/events/{id}/fcas/{fca_id}", {
          params: { path: { id: eventId, fca_id: id } },
        });
        if (error) throw new Error("delete failed");
        return;
      }
      const { error } = await ois.DELETE("/api/v1/flow/fcas/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: fcaKey(eventId) });
      toast.success("FCA deleted");
    },
    onError: () => toast.error("Couldn’t delete the FCA"),
  });
}

/** Publish an event FCA (planned → published) so it goes live on every map. */
export function usePublishEventFca(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (fcaId: string) => {
      const { data, error } = await ois.POST("/api/v1/events/{id}/fcas/{fca_id}/publish", {
        params: { path: { id: eventId, fca_id: fcaId } },
      });
      if (error || !data) throw new Error("publish failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: fcaKey(eventId) });
      queryClient.invalidateQueries({ queryKey: ["fcas"] }); // now on the live maps
      toast.success("FCA published");
    },
    onError: () => toast.error("Couldn’t publish the FCA"),
  });
}

/** Archive an event FCA (planned/published → archived), pulling it off the live maps. */
export function useArchiveEventFca(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (fcaId: string) => {
      const { data, error } = await ois.POST("/api/v1/events/{id}/fcas/{fca_id}/archive", {
        params: { path: { id: eventId, fca_id: fcaId } },
      });
      if (error || !data) throw new Error("archive failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: fcaKey(eventId) });
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
      toast.success("FCA archived");
    },
    onError: () => toast.error("Couldn’t archive the FCA"),
  });
}

/** Toggle whether an event FCA auto-publishes 30 min before the event starts. */
export function useSetEventFcaAuto(eventId: number) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ fcaId, auto }: { fcaId: string; auto: boolean }) => {
      const { data, error } = await ois.PUT("/api/v1/events/{id}/fcas/{fca_id}/auto", {
        params: { path: { id: eventId, fca_id: fcaId } },
        body: { auto_publish: auto },
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: fcaKey(eventId) }),
    onError: () => toast.error("Couldn’t change auto-publish"),
  });
}

/** An aircraft's filed route (lat/lon anchors) — fetched on demand when clicked. */
export function useAircraftRoute(callsign: string | null) {
  return useQuery({
    queryKey: ["aircraft-route", callsign],
    queryFn: async () => {
      const { data, error } = await ois.GET(
        "/api/v1/flow/aircraft/{callsign}/route",
        { params: { path: { callsign: callsign! } } },
      );
      if (error || !data) throw new Error("failed to load route");
      return data;
    },
    enabled: !!callsign,
    staleTime: 30_000,
  });
}

/** Matched-aircraft counts per FCA (all FCAs), refreshed every 15s. */
export function useFcaCounts() {
  return useQuery({
    queryKey: ["fca-counts"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/counts");
      if (error || !data) throw new Error("failed to load counts");
      return data as Record<string, number>;
    },
    refetchInterval: 15_000,
  });
}

/** VATSIM traffic for the map. Live (15s poll) by default; inside a `HistoricalProvider` it
 * reconstructs the network at the scrubber instant, so the embedded map widget replays. */
export function useTraffic() {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["flow-traffic"] : ["hist-traffic", at],
    queryFn: async () => {
      if (at != null) return fetchHistTraffic(at);
      const { data, error } = await ois.GET("/api/v1/flow/traffic");
      if (error || !data) throw new Error("failed to load traffic");
      return data;
    },
    refetchInterval: at == null ? 15_000 : false,
    staleTime: at == null ? 0 : Infinity,
    placeholderData: at == null ? undefined : keepPreviousData,
  });
}

async function fetchHistAtc(at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/atc", {
    params: { query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical ATC");
  return data;
}

/** Online ATC (badges + TRACON areas + centers) for the map ATC layer. Only polls while the layer
 * is enabled; matches the traffic layer's 15s cadence so logon/logoff appears promptly. Inside a
 * `HistoricalProvider` it reconstructs the online ATC at the scrubber instant. */
export function useAtc(enabled: boolean) {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["flow-atc"] : ["hist-atc", at],
    queryFn: async () => {
      if (at != null) return fetchHistAtc(at);
      const { data, error } = await ois.GET("/api/v1/flow/atc");
      if (error || !data) throw new Error("failed to load ATC");
      return data;
    },
    enabled,
    refetchInterval: at == null ? 15_000 : false,
    staleTime: at == null ? 0 : Infinity,
    placeholderData: at == null ? undefined : keepPreviousData,
  });
}

/** Aircraft whose filed route crosses one FCA, with ETA to the crossing. */
export function useFcaTraffic(id: string | null, debug = false) {
  return useQuery({
    queryKey: ["fca-traffic", id, debug],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/fcas/{id}/traffic", {
        params: { path: { id: id! }, query: { debug: debug || undefined } },
      });
      if (error || !data) throw new Error("failed to load FCA traffic");
      return data;
    },
    enabled: !!id,
    // Releases sync instantly over the websocket; the poll refreshes live crossing ETAs (and is the
    // fallback if the socket drops).
    refetchInterval: 30_000,
  });
}

/** Matched/sequenced traffic for several FCAs at once (the ARTCC overview). Each query shares its
 *  cache key with {@link useFcaTraffic}, so opening one FCA's detail reuses the fetched data. */
export function useFcaTrafficMany(ids: string[]) {
  return useQueries({
    queries: ids.map((id) => ({
      queryKey: ["fca-traffic", id],
      queryFn: async () => {
        const { data, error } = await ois.GET("/api/v1/flow/fcas/{id}/traffic", {
          params: { path: { id } },
        });
        if (error || !data) throw new Error("failed to load FCA traffic");
        return data;
      },
      refetchInterval: 30_000,
    })),
  });
}

/** Issue a CFR release (RDY = earliest slot; `ready` HHMMz = pinned wheels-up). */
export function useMarkRelease(fcaId: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({
      callsign,
      ready,
    }: {
      callsign: string;
      ready?: string;
    }) => {
      const { data, error } = await ois.POST(
        "/api/v1/flow/fcas/{id}/release/{callsign}",
        { params: { path: { id: fcaId, callsign } }, body: { ready } },
      );
      if (error || !data) throw new Error("release failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["fca-traffic", fcaId], data);
      // A release also shows in the IDST board + airport departures — refresh them in this session
      // instead of waiting for their next poll.
      queryClient.invalidateQueries({ queryKey: ["idst"] });
      queryClient.invalidateQueries({ queryKey: ["departures"] });
    },
    onError: () => toast.error("Couldn’t issue the release"),
  });
}

export function useClearRelease(fcaId: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (callsign: string) => {
      const { data, error } = await ois.DELETE(
        "/api/v1/flow/fcas/{id}/release/{callsign}",
        { params: { path: { id: fcaId, callsign } } },
      );
      if (error || !data) throw new Error("clear failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["fca-traffic", fcaId], data);
      queryClient.invalidateQueries({ queryKey: ["idst"] });
      queryClient.invalidateQueries({ queryKey: ["departures"] });
    },
    onError: () => toast.error("Couldn’t clear the release"),
  });
}

/** Set the manual crossing order (empty array resets to auto). */
export function useReorderFca(fcaId: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const key = ["fca-traffic", fcaId] as const;
  return useMutation({
    mutationFn: async (order: string[]) => {
      const { error } = await ois.PUT("/api/v1/flow/fcas/{id}/order", {
        params: { path: { id: fcaId } },
        body: { order },
      });
      if (error) throw new Error("reorder failed");
    },
    // Optimistically apply the new order so the dropped row stays put instead of snapping back
    // while the server round-trips; reconciled by the invalidate in onSettled.
    onMutate: async (order: string[]) => {
      await queryClient.cancelQueries({ queryKey: key });
      const prev = queryClient.getQueryData<FcaFlight[]>(key);
      if (prev) {
        const rank = new Map(order.map((cs, i) => [cs, i]));
        const next = [...prev]
          .sort(
            (a, b) =>
              (rank.get(a.callsign) ?? Number.MAX_SAFE_INTEGER) -
              (rank.get(b.callsign) ?? Number.MAX_SAFE_INTEGER),
          )
          .map((f, i) => ({ ...f, seq: i + 1 }));
        queryClient.setQueryData(key, next);
      }
      return { prev };
    },
    onError: (_e, _order, ctx) => {
      if (ctx?.prev) queryClient.setQueryData(key, ctx.prev);
      toast.error("Couldn’t reorder");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: key });
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
    },
  });
}

/** Build an upsert payload from an existing FCA (for toggles / edits). */
export function toUpsert(fca: Fca): UpsertFca {
  return {
    name: fca.name,
    color: fca.color,
    artcc: fca.artcc,
    points: fca.points,
    dests: fca.dests,
    origins: fca.origins,
    fixes: fca.fixes,
    scope: fca.scope,
    min_fl: fca.min_fl,
    max_fl: fca.max_fl,
    dir: fca.dir,
    mode: fca.mode,
    rate: fca.rate,
    mit: fca.mit,
    enabled: fca.enabled,
  };
}
