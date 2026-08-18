import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
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

/** Force an immediate nav + winds refresh (requires flow.fca.update). */
export function useRefreshData() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await ois.POST("/api/v1/flow/data-refresh");
      if (error || !data) throw new Error("refresh failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["flow-data-status"], data);
      // Freshly resolved routes may shift matches/ETAs.
      queryClient.invalidateQueries({ queryKey: ["fca-traffic"] });
      queryClient.invalidateQueries({ queryKey: ["aircraft-route"] });
      toast.success(`Nav ${data.nav_cycle} · ${data.winds_stations} wind stations`);
    },
    onError: () => toast.error("Refresh failed"),
  });
}

/** All FCAs (shared across controllers). */
export function useFcas() {
  return useQuery({
    queryKey: ["fcas"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/fcas");
      if (error || !data) throw new Error("failed to load FCAs");
      return data;
    },
  });
}

export function useCreateFca() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertFca) => {
      const { data, error } = await ois.POST("/api/v1/flow/fcas", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
      toast.success("FCA created");
    },
    onError: () => toast.error("Couldn’t create the FCA"),
  });
}

export function useUpdateFca() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertFca }) => {
      const { data, error } = await ois.PUT("/api/v1/flow/fcas/{id}", {
        params: { path: { id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
    },
    onError: () => toast.error("Couldn’t save the FCA"),
  });
}

export function useDeleteFca() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/flow/fcas/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
      toast.success("FCA deleted");
    },
    onError: () => toast.error("Couldn’t delete the FCA"),
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
 * is enabled; the geometry is heavier than traffic so it refreshes every 30s. Inside a
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
    refetchInterval: at == null ? 30_000 : false,
    staleTime: at == null ? 0 : Infinity,
  });
}

/** Aircraft whose filed route crosses one FCA, with ETA to the crossing. */
export function useFcaTraffic(id: string | null) {
  return useQuery({
    queryKey: ["fca-traffic", id],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/fcas/{id}/traffic", {
        params: { path: { id: id! } },
      });
      if (error || !data) throw new Error("failed to load FCA traffic");
      return data;
    },
    enabled: !!id,
    refetchInterval: 15_000,
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
    },
    onError: () => toast.error("Couldn’t clear the release"),
  });
}

/** Set the manual crossing order (empty array resets to auto). */
export function useReorderFca(fcaId: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (order: string[]) => {
      const { error } = await ois.PUT("/api/v1/flow/fcas/{id}/order", {
        params: { path: { id: fcaId } },
        body: { order },
      });
      if (error) throw new Error("reorder failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["fca-traffic", fcaId] });
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
    },
    onError: () => toast.error("Couldn’t reorder"),
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
