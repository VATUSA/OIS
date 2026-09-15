import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type AirportSurface = components["schemas"]["AirportSurfaceBody"];
export type AirportGate = components["schemas"]["AirportGateBody"];
export type AirportRampArea = components["schemas"]["AirportRampAreaBody"];
export type AirportTaxiway = components["schemas"]["AirportTaxiwayBody"];
export type AirportRunway = components["schemas"]["AirportRunwayBody"];
export type UpsertAirportGate = components["schemas"]["UpsertAirportGateRequest"];
export type UpsertAirportRampArea = components["schemas"]["UpsertAirportRampAreaRequest"];
export type UpsertAirportTaxiway = components["schemas"]["UpsertAirportTaxiwayRequest"];
export type UpsertAirportRunway = components["schemas"]["UpsertAirportRunwayRequest"];
export type FaaRepullResult = components["schemas"]["FaaRepullResult"];

const key = (icao: string) => ["airport-surface", icao] as const;

/** An airport's full surface geometry (gates, ramp/apron areas, taxiways, runways) for the editor. */
export function useAirportSurface(icao: string | null) {
  return useQuery({
    queryKey: key(icao ?? ""),
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/airports/{icao}/surface", {
        params: { path: { icao: icao! } },
      });
      if (error || !data) throw new Error("failed to load airport surface data");
      return data;
    },
    enabled: !!icao,
  });
}

export function useCreateAirportGate(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertAirportGate) => {
      const { data, error } = await ois.POST("/api/v1/airports/{icao}/gates", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t add the gate"),
  });
}

export function useUpdateAirportGate(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertAirportGate }) => {
      const { data, error } = await ois.PUT("/api/v1/airports/{icao}/gates/{id}", {
        params: { path: { icao, id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t save the gate"),
  });
}

export function useDeleteAirportGate(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/airports/{icao}/gates/{id}", {
        params: { path: { icao, id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t delete the gate"),
  });
}

export function useCreateAirportRampArea(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertAirportRampArea) => {
      const { data, error } = await ois.POST("/api/v1/airports/{icao}/ramp-areas", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t add the ramp/apron area"),
  });
}

export function useUpdateAirportRampArea(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertAirportRampArea }) => {
      const { data, error } = await ois.PUT("/api/v1/airports/{icao}/ramp-areas/{id}", {
        params: { path: { icao, id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t save the ramp/apron area"),
  });
}

export function useDeleteAirportRampArea(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/airports/{icao}/ramp-areas/{id}", {
        params: { path: { icao, id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t delete the ramp/apron area"),
  });
}

export function useCreateAirportTaxiway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertAirportTaxiway) => {
      const { data, error } = await ois.POST("/api/v1/airports/{icao}/taxiways", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t add the taxiway"),
  });
}

export function useUpdateAirportTaxiway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertAirportTaxiway }) => {
      const { data, error } = await ois.PUT("/api/v1/airports/{icao}/taxiways/{id}", {
        params: { path: { icao, id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t save the taxiway"),
  });
}

export function useDeleteAirportTaxiway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/airports/{icao}/taxiways/{id}", {
        params: { path: { icao, id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t delete the taxiway"),
  });
}

export function useCreateAirportRunway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertAirportRunway) => {
      const { data, error } = await ois.POST("/api/v1/airports/{icao}/runways", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t add the runway"),
  });
}

export function useUpdateAirportRunway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertAirportRunway }) => {
      const { data, error } = await ois.PUT("/api/v1/airports/{icao}/runways/{id}", {
        params: { path: { icao, id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t save the runway"),
  });
}

export function useDeleteAirportRunway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/airports/{icao}/runways/{id}", {
        params: { path: { icao, id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: key(icao) }),
    onError: () => toast.error("Couldn’t delete the runway"),
  });
}

/** Re-pull this airport's FAA-sourced ramp/taxiway/runway rows from the current bundled extract (#232) —
 * an on-demand equivalent of #231's nationwide startup seed, scoped to one airport. */
export function useRepullFaaSurface(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async () => {
      const { data, error, response } = await ois.POST(
        "/api/v1/airports/{icao}/surface/repull-faa",
        { params: { path: { icao } } },
      );
      if (response.status === 404) throw new Error("uncovered");
      if (error || !data) throw new Error("repull failed");
      return data;
    },
    onSuccess: (data: FaaRepullResult) => {
      queryClient.invalidateQueries({ queryKey: key(icao) });
      toast.success("Re-pulled FAA surface data", {
        description: `${data.taxiways_inserted} taxiways, ${data.ramps_inserted} ramps, ${data.runways_inserted} runways`,
      });
    },
    onError: (e) =>
      e instanceof Error && e.message === "uncovered"
        ? toast.error("No FAA data for this airport", {
            description: "The bundled FAA extract doesn’t cover it — nothing was changed.",
          })
        : toast.error("Couldn’t re-pull FAA surface data"),
  });
}
