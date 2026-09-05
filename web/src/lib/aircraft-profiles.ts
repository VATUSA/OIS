import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type AircraftProfile = components["schemas"]["AircraftProfileBody"];
export type UpsertAircraftProfile = components["schemas"]["UpsertAircraftProfileRequest"];

/** Configurable per-aircraft performance profiles for the trajectory / ETA model. */
export function useAircraftProfiles() {
  return useQuery({
    queryKey: ["aircraft-profiles"],
    queryFn: async (): Promise<AircraftProfile[]> => {
      const { data, error } = await ois.GET("/api/v1/flow/aircraft-profiles");
      if (error || !data) throw new Error("failed to load aircraft profiles");
      return data;
    },
  });
}

export function useUpsertAircraftProfile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: {
      kind: string;
      key: string;
      body: UpsertAircraftProfile;
    }): Promise<AircraftProfile> => {
      const { data, error } = await ois.PUT("/api/v1/flow/aircraft-profiles/{kind}/{key}", {
        params: { path: { kind: input.kind, key: input.key } },
        body: input.body,
      });
      if (error || !data) throw new Error("save aircraft profile failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["aircraft-profiles"] }),
  });
}

export function useDeleteAircraftProfile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: { kind: string; key: string }) => {
      const { error } = await ois.DELETE("/api/v1/flow/aircraft-profiles/{kind}/{key}", {
        params: { path: { kind: input.kind, key: input.key } },
      });
      if (error) throw new Error("delete aircraft profile failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["aircraft-profiles"] }),
  });
}

// --- SimBrief-style speed-schedule formatting (250/280/.78, .78/280/250) ---

/** Format a Mach like `0.78` as `.78` for a schedule string. */
function machStr(m: number | null | undefined): string | null {
  if (m == null) return null;
  return m.toFixed(2).replace(/^0/, "").replace(/0$/, "");
}

/** Parse a Mach token: `.78`, `0.78`, `78`, or `M78` → 0.78. Returns null if empty/invalid. */
function parseMach(tok: string | undefined): number | null {
  if (!tok) return null;
  const t = tok.replace(/[mM]/g, "").trim();
  if (t === "") return null;
  let n = Number(t);
  if (!Number.isFinite(n)) return null;
  if (n >= 1) n = n / 100; // "78" → 0.78
  return n > 0 && n < 1 ? Math.round(n * 100) / 100 : null;
}

/** Climb schedule as `IASlo/IAShi[/Mach]`, e.g. `250/280/.78`. */
export function formatClimb(p: {
  climb_ias_lo: number;
  climb_ias_hi: number;
  climb_mach?: number | null;
}): string {
  const m = machStr(p.climb_mach);
  return `${Math.round(p.climb_ias_lo)}/${Math.round(p.climb_ias_hi)}${m ? `/${m}` : ""}`;
}

/** Descent schedule as `[Mach/]IAShi/IASlo`, e.g. `.78/280/250`. */
export function formatDescent(p: {
  desc_ias_hi: number;
  desc_ias_lo: number;
  desc_mach?: number | null;
}): string {
  const m = machStr(p.desc_mach);
  return `${m ? `${m}/` : ""}${Math.round(p.desc_ias_hi)}/${Math.round(p.desc_ias_lo)}`;
}

/** Parse a climb schedule `250/280/.78` → {ias_lo, ias_hi, mach}. Missing parts left undefined. */
export function parseClimb(
  s: string,
): { ias_lo?: number; ias_hi?: number; mach?: number | null } {
  const parts = s.split("/").map((x) => x.trim());
  return {
    ias_lo: parts[0] ? Number(parts[0]) : undefined,
    ias_hi: parts[1] ? Number(parts[1]) : undefined,
    mach: parts.length > 2 ? parseMach(parts[2]) : undefined,
  };
}

/** Parse a descent schedule `.78/280/250` (mach optional first) → {ias_hi, ias_lo, mach}. */
export function parseDescent(
  s: string,
): { ias_hi?: number; ias_lo?: number; mach?: number | null } {
  const parts = s.split("/").map((x) => x.trim());
  // Three parts → Mach/IAShi/IASlo; two parts → IAShi/IASlo (no Mach).
  if (parts.length >= 3) {
    return { mach: parseMach(parts[0]), ias_hi: Number(parts[1]), ias_lo: Number(parts[2]) };
  }
  return {
    mach: null,
    ias_hi: parts[0] ? Number(parts[0]) : undefined,
    ias_lo: parts[1] ? Number(parts[1]) : undefined,
  };
}

/** Human label for a profile row's identity. */
export function profileLabel(p: AircraftProfile): string {
  if (p.kind === "default") return "Default (all other aircraft)";
  if (p.kind === "wake") return `Wake class ${p.key}`;
  return p.key;
}
