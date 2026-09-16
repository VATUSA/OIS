import {useEffect, useRef, useState} from "react";
import type {PickingInfo} from "@deck.gl/core";
import {Plus} from "lucide-react";
import {Button} from "@ois/ui";

import {
  type AirportSurface,
  useCreateAirportGate,
  useCreateAirportRampArea,
  useCreateAirportRunway,
  useCreateAirportTaxiway,
  useDeleteAirportGate,
  useDeleteAirportRampArea,
  useDeleteAirportRunway,
  useDeleteAirportTaxiway,
  useUpdateAirportGate,
  useUpdateAirportRampArea,
  useUpdateAirportRunway,
  useUpdateAirportTaxiway,
} from "@/lib/airport-surface";

import {MapCanvas} from "../MapCanvas";
import {useMapCamera} from "../hooks/useMapCamera";
import {haversine, normPoints, toDeckPath, type LatLng} from "../lib/geo";
import {SurfaceEditorPanel, type SurfaceDraft} from "./editor-panel";
import {MIN_SURFACE_POINTS, buildSurfaceDraftLayers, buildSurfaceLayers, type SurfaceKind} from "./layers";

const KINDS: { kind: SurfaceKind; label: string }[] = [
  { kind: "gate", label: "Add gate" },
  { kind: "taxiway", label: "Add taxiway" },
  { kind: "ramp", label: "Add ramp/apron" },
  { kind: "runway", label: "Add runway" },
];

function layerIdToKind(id: string | undefined): SurfaceKind | null {
  if (id === "surface-gates") return "gate";
  if (id === "surface-taxiways") return "taxiway";
  if (id === "surface-ramp-areas") return "ramp";
  if (id === "surface-runways") return "runway";
  return null;
}

/** The airport surface data editor's map: draws/edits gates, taxiways, runways, and ramp/apron areas over
 * the OSM airport-layout basemap. Built directly on the generic `MapCanvas` shell (not
 * `TrafficMap`, which drags in unrelated traffic/ATC props) — the draw state machine mirrors the
 * FCA polyline editor (`FcaMapView.tsx`'s `Draft`/`Phase`), extended to point and polygon shapes. */
export function SurfaceMap({
  icao,
  surface,
  editable,
}: {
  icao: string;
  surface: AirportSurface;
  editable: boolean;
}) {
  const camera = useMapCamera();
  const [draft, setDraft] = useState<SurfaceDraft | null>(null);
  const [phase, setPhase] = useState<"draw" | "edit">("draw");
  const dragIndex = useRef<number | null>(null);
  const [draggingVertex, setDraggingVertex] = useState(false);
  // The double-click-finish timer (see handleClick). Reset in startNew/startEditExisting so a
  // timestamp from finishing one draft can't make the very next draft's first click look doubled.
  const lastClickT = useRef(0);

  const createGate = useCreateAirportGate(icao);
  const updateGate = useUpdateAirportGate(icao);
  const deleteGate = useDeleteAirportGate(icao);
  const createRamp = useCreateAirportRampArea(icao);
  const updateRamp = useUpdateAirportRampArea(icao);
  const deleteRamp = useDeleteAirportRampArea(icao);
  const createTaxiway = useCreateAirportTaxiway(icao);
  const updateTaxiway = useUpdateAirportTaxiway(icao);
  const deleteTaxiway = useDeleteAirportTaxiway(icao);
  const createRunway = useCreateAirportRunway(icao);
  const updateRunway = useUpdateAirportRunway(icao);
  const deleteRunway = useDeleteAirportRunway(icao);

  const pending =
    createGate.isPending ||
    updateGate.isPending ||
    deleteGate.isPending ||
    createRamp.isPending ||
    updateRamp.isPending ||
    deleteRamp.isPending ||
    createTaxiway.isPending ||
    updateTaxiway.isPending ||
    deleteTaxiway.isPending ||
    createRunway.isPending ||
    updateRunway.isPending ||
    deleteRunway.isPending;

  // No airport-lookup source exists to center the map on `icao` directly (runway ends have no
  // lat/lon anywhere in this codebase). Once, on first load, fly to the loaded geometry's bounds
  // instead — a no-op (stays at the default CONUS view) for a brand-new airport with no data yet.
  const centered = useRef(false);
  useEffect(() => {
    if (centered.current) return;
    const pts: [number, number][] = [
      ...toDeckPath(surface.gates.map((g): LatLng => [g.lat, g.lon])),
      ...[...surface.taxiways, ...surface.runways, ...surface.ramp_areas].flatMap((p) =>
        p.rings.flatMap((ring) => toDeckPath(ring as LatLng[])),
      ),
    ];
    if (pts.length > 0) {
      centered.current = true;
      camera.fitBounds(pts, { padding: 60, maxZoom: 16 });
    }
    // `camera.fitBounds` is a stable ref-backed callback (see useMapCamera) — safe to omit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [surface]);

  const startNew = (kind: SurfaceKind) => {
    // A stale timestamp from finishing a *previous* draft's double-click must not make this
    // draft's very first map click look like a double-click too (see lastClickT's declaration).
    lastClickT.current = 0;
    setDraft({ kind, name: "", rampKind: "apron", points: [] });
    setPhase("draw");
  };

  const startEditExisting = (kind: SurfaceKind, id: string) => {
    lastClickT.current = 0;
    if (kind === "gate") {
      const g = surface.gates.find((x) => x.id === id);
      if (!g) return;
      setDraft({ kind, id, name: g.name, rampKind: "apron", points: [[g.lat, g.lon]] });
    } else {
      const r =
        kind === "taxiway"
          ? surface.taxiways.find((x) => x.id === id)
          : kind === "runway"
            ? surface.runways.find((x) => x.id === id)
            : surface.ramp_areas.find((x) => x.id === id);
      if (!r) return;
      const outer = r.rings[0] ?? [];
      // Strip the closing duplicate — the draft never carries it (see layers.ts).
      const open = outer.length > 1 && haversine(outer[0] as LatLng, outer[outer.length - 1] as LatLng) < 0.01
        ? outer.slice(0, -1)
        : outer;
      // This editor only draws/edits the outer ring — any further rings (e.g. a hole) are carried
      // through untouched so save() can resend them rather than silently dropping them.
      const extraRings = r.rings.slice(1) as LatLng[][];
      setDraft({
        kind,
        id,
        name: r.name,
        rampKind: "kind" in r && r.kind === "ramp" ? "ramp" : "apron",
        points: open as LatLng[],
        extraRings,
      });
    }
    setPhase("edit");
  };

  const addVertex = ([lon, lat]: [number, number]) =>
    setDraft((d) => {
      if (!d) return d;
      const p: LatLng = [lat, lon];
      const last = d.points[d.points.length - 1];
      // Dedupe only near-identical repeated clicks (e.g. a double-click whose two events land a
      // pixel apart, missing the time-based check above) — 0.001nm (~1.85m) is far below any
      // legitimate spacing between real airport-surface vertices (adjacent gates, tight taxiway
      // curves), unlike the previous 0.05nm (~92m) threshold, which silently swallowed those.
      if (last && haversine(last, p) < 0.001) return d;
      const points = [...d.points, p];
      // A gate is a single point — placing it finalizes the shape immediately.
      if (d.kind === "gate") setPhase("edit");
      return { ...d, points };
    });

  const moveVertex = (index: number, [lon, lat]: [number, number]) =>
    setDraft((d) => {
      if (!d) return d;
      const points = d.points.slice();
      points[index] = [lat, lon];
      return { ...d, points };
    });

  const finishDraw = () =>
    setDraft((d) => {
      if (!d || d.points.length < MIN_SURFACE_POINTS[d.kind]) return d;
      setPhase("edit");
      return d;
    });

  const cancel = () => setDraft(null);

  const save = () => {
    if (!draft || !draft.name.trim()) return;
    const points = normPoints(draft.points);
    const onDone = () => setDraft(null);
    if (draft.kind === "gate") {
      const body = { name: draft.name.trim(), lat: points[0][0], lon: points[0][1] };
      if (draft.id) updateGate.mutate({ id: draft.id, body }, { onSuccess: onDone });
      else createGate.mutate(body, { onSuccess: onDone });
    } else {
      // Any rings beyond the outer one (e.g. a hole) came from an existing row this editor doesn't
      // draw — resend them unchanged rather than silently dropping them (see startEditExisting).
      const rings = [[...points, points[0]], ...(draft.extraRings ?? [])];
      if (draft.kind === "taxiway") {
        const body = { name: draft.name.trim(), rings };
        if (draft.id) updateTaxiway.mutate({ id: draft.id, body }, { onSuccess: onDone });
        else createTaxiway.mutate(body, { onSuccess: onDone });
      } else if (draft.kind === "runway") {
        const body = { name: draft.name.trim(), rings };
        if (draft.id) updateRunway.mutate({ id: draft.id, body }, { onSuccess: onDone });
        else createRunway.mutate(body, { onSuccess: onDone });
      } else {
        const body = { name: draft.name.trim(), kind: draft.rampKind, rings };
        if (draft.id) updateRamp.mutate({ id: draft.id, body }, { onSuccess: onDone });
        else createRamp.mutate(body, { onSuccess: onDone });
      }
    }
  };

  const deleteCurrent = () => {
    if (!draft?.id) return;
    const onDone = () => setDraft(null);
    if (draft.kind === "gate") deleteGate.mutate(draft.id, { onSuccess: onDone });
    else if (draft.kind === "taxiway") deleteTaxiway.mutate(draft.id, { onSuccess: onDone });
    else if (draft.kind === "runway") deleteRunway.mutate(draft.id, { onSuccess: onDone });
    else deleteRamp.mutate(draft.id, { onSuccess: onDone });
  };

  // Keyboard shortcuts while a draft is open, mirroring the FCA polyline editor: Escape cancels
  // from either phase; Enter finishes drawing and Backspace drops the last point, both only while
  // still placing points (the name panel that follows has no map-focused shortcuts of its own).
  useEffect(() => {
    if (!draft) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        cancel();
      } else if (phase === "draw" && draft.kind !== "gate") {
        if (e.key === "Enter") finishDraw();
        else if (e.key === "Backspace" && draft.points.length > 0) {
          e.preventDefault();
          setDraft((d) => (d ? { ...d, points: d.points.slice(0, -1) } : d));
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [draft, phase]);

  const layers = [
    ...buildSurfaceLayers(surface, draft?.id ? { kind: draft.kind, id: draft.id } : null),
    ...(draft ? buildSurfaceDraftLayers(draft.kind, draft.points, phase) : []),
  ];

  const handleClick = (info: PickingInfo, event: unknown) => {
    // Once a shape is finalized (phase "edit" — reached via Finish/Close-shape, a double-click, or
    // opening an existing shape to edit), a stray map click must not silently append another vertex:
    // only actively placing points (phase "draw") should react to clicks at all.
    if (draft && phase === "draw") {
      if (info.layer?.id === "surface-draft-vertices") return; // a click meant to grab a handle
      if (!info.coordinate) return;
      if (draft.kind === "gate") {
        if (draft.points.length === 0) addVertex(info.coordinate as [number, number]);
        return; // a gate is placed on its first click — further clicks are no-ops until saved
      }
      // Detect a double-click (deck has no onDblClick); it finishes the shape rather than adding
      // one more vertex at the same spot, matching the FCA polyline editor's convention.
      const t = (event as { srcEvent?: { timeStamp?: number } })?.srcEvent?.timeStamp ?? performance.now();
      const isDouble = t - lastClickT.current < 300;
      lastClickT.current = t;
      if (isDouble) {
        finishDraw();
        return;
      }
      addVertex(info.coordinate as [number, number]);
      return;
    }
    if (draft) return; // phase "edit": dragging handles is the only interaction, handled separately
    if (!editable) return;
    const kind = layerIdToKind(info.layer?.id);
    const id = (info.object as { id?: string } | undefined)?.id;
    if (kind && id) startEditExisting(kind, id);
  };

  const handleDragStart = (info: PickingInfo, event: unknown) => {
    if (draft && info.layer?.id === "surface-draft-vertices" && info.index != null && info.index >= 0) {
      dragIndex.current = info.index;
      setDraggingVertex(true);
      (event as { stopPropagation?: () => void })?.stopPropagation?.();
    }
  };
  const handleDrag = (info: PickingInfo) => {
    if (dragIndex.current != null && info.coordinate) {
      moveVertex(dragIndex.current, info.coordinate as [number, number]);
    }
  };
  const handleDragEnd = () => {
    dragIndex.current = null;
    setDraggingVertex(false);
  };

  const controller = draft
    ? { doubleClickZoom: false, dragPan: !draggingVertex, dragRotate: false }
    : true;

  return (
    <div className="relative h-[70vh] w-full overflow-hidden rounded-md border">
      <MapCanvas
        viewState={camera.viewState}
        onViewStateChange={camera.onViewStateChange}
        onResize={camera.onResize}
        controller={controller}
        layers={layers}
        onClick={handleClick}
        onDragStart={handleDragStart}
        onDrag={handleDrag}
        onDragEnd={handleDragEnd}
        getCursor={({ isHovering }) => (draft ? "crosshair" : isHovering ? "pointer" : "grab")}
      >
        {editable && !draft && (
          <div className="absolute left-3 top-3 flex flex-col gap-2">
            {KINDS.map(({ kind, label }) => (
              <Button key={kind} size="sm" variant="secondary" onClick={() => startNew(kind)}>
                <Plus className="size-4" />
                {label}
              </Button>
            ))}
          </div>
        )}
        {draft && (
          <div className="absolute right-3 top-3 w-72">
            <SurfaceEditorPanel
              draft={draft}
              phase={phase}
              pending={pending}
              onNameChange={(name) => setDraft((d) => (d ? { ...d, name } : d))}
              onRampKindChange={(rampKind) => setDraft((d) => (d ? { ...d, rampKind } : d))}
              onFinishDraw={finishDraw}
              onSave={save}
              onCancel={cancel}
              onDelete={draft.id ? deleteCurrent : undefined}
            />
          </div>
        )}
      </MapCanvas>
    </div>
  );
}
