import {useEffect, useMemo, useRef, useState} from "react";
import {Button, ConfirmButton, Input, Switch, useTheme, useToast} from "@ois/ui";
import {closestCenter, DndContext, type DragEndEvent, PointerSensor, useSensor, useSensors} from "@dnd-kit/core";
import {arrayMove, SortableContext, useSortable, verticalListSortingStrategy} from "@dnd-kit/sortable";
import {CSS} from "@dnd-kit/utilities";
import {ArrowLeft, Eye, EyeOff, GripVertical, Home, Menu, Pencil, Plane, Plus, RadioTower, Tag, Trash2, X} from "lucide-react";
import {Link} from "@tanstack/react-router";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {
  toUpsert,
  useAircraftRoute,
  useAtc,
  useCreateFca,
  useDataStatus,
  useDeleteFca,
  useFcaCounts,
  useFcas,
  useFcaTraffic,
  useFcaTrafficMany,
  useTraffic,
  useUpdateFca,
  type Fca,
  type UpsertFca,
} from "@/lib/fca";
import {useCreateRoute, useDeleteRoute, useRoutes, useUpdateRoute, type MapRoute, type UpsertRoute} from "@/lib/route";
import {MAX_PROJECTION_SEC, usePredictedTraffic} from "@/lib/prediction";
import {useSetting} from "@/lib/settings";
import {FlightSearch} from "@/components/flight-search";
import {FcaDetail} from "@/pages/fca/detail";
import {FcaOverviewPanel, type OverviewGroup} from "@/pages/fca/overview-panel";
import {facilityFeature, facilityPoints} from "@/lib/facility-map/boundary";
import boundariesGeo from "@/assets/artcc-boundaries.json";

import {TrafficMap} from "./TrafficMap";
import {useMapCamera} from "./hooks/useMapCamera";
import {usePersistedOrder} from "./hooks/usePersistedOrder";
import {aircraftColor, HIGHLIGHT} from "./lib/colors";
import {US_HOME} from "./lib/constants";
import {haversine, normPoints, toDeckPath, type LatLng} from "./lib/geo";
import type {NormAircraft} from "./lib/types";
import type {AtcData} from "./layers/atc";
import type {MatchedFlight} from "./layers/matched";
import type {NamedRoute} from "./layers/routes";
import type {MapFca} from "./layers/fca";
import {RoutePopup} from "./fca/RoutePopup";
import {CoveragePanel} from "./fca/CoveragePanel";
import {
  blankDraft,
  blankRouteForm,
  DraftEditor,
  draftFrom,
  Kbd,
  parseFl,
  parseList,
  RouteEditor,
  routeFormFrom,
  type Draft,
  type Phase,
  type RouteForm,
} from "./fca/editors";

const BOUNDARIES = boundariesGeo as GeoJSON.FeatureCollection;
/** deck initial camera framing the CONUS (matches the legacy US_HOME zoom ~4.3 at zoom 3.9). */
const FCA_INITIAL = { longitude: US_HOME.longitude, latitude: US_HOME.latitude, zoom: 3.9 };

function cycleAgeDays(cycle: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(cycle);
  if (!m) return null;
  const d = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return Math.floor((Date.now() - d) / 86_400_000);
}

/** One draggable row in the sidebar FCA list (see #109 — order is per-viewer, via `usePersistedOrder`). */
function FcaRow({
  fca,
  selected,
  count,
  canEdit,
  canDelete,
  onSelect,
  onToggleEnabled,
  onEdit,
  onDelete,
}: {
  fca: Fca;
  selected: boolean;
  count: number;
  canEdit: boolean;
  canDelete: boolean;
  onSelect: () => void;
  onToggleEnabled: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: fca.id,
  });
  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : fca.enabled ? 1 : 0.55,
  };
  return (
    <li
      ref={setNodeRef}
      style={style}
      className={"flex items-center gap-2 border-b px-3 py-2 text-sm " + (selected ? "bg-accent/40" : "")}
    >
      <button
        type="button"
        className="cursor-grab text-muted-foreground/60 hover:text-foreground"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-3.5" />
      </button>
      <span
        className="size-3 shrink-0 rounded-full"
        style={{ background: fca.color }}
        title={fca.enabled ? "Enabled" : "Disabled"}
      />
      <button type="button" onClick={onSelect} className="flex-1 truncate text-left font-mono">
        {fca.name}
        {fca.artcc && <span className="ml-1.5 text-xs text-muted-foreground">{fca.artcc}</span>}
      </button>
      <span
        className={
          "shrink-0 rounded px-1.5 text-xs font-medium tabular-nums " +
          (count > 0 ? "bg-primary/15 text-primary" : "text-muted-foreground/50")
        }
      >
        {count}
      </span>
      {canEdit && (
        <span
          className="flex shrink-0 items-center"
          title={fca.enabled ? "Enabled — click to disable" : "Disabled — click to enable"}
        >
          <Switch
            checked={fca.enabled}
            onCheckedChange={onToggleEnabled}
            aria-label={`${fca.enabled ? "Disable" : "Enable"} the ${fca.name} FCA`}
            className="scale-[0.68]"
          />
        </span>
      )}
      {canEdit && (
        <button type="button" title="Edit" onClick={onEdit} className="text-muted-foreground hover:text-foreground">
          <Pencil className="size-3.5" />
        </button>
      )}
      {canDelete && (
        <ConfirmButton
          size="icon"
          className="size-7"
          title="Delete"
          aria-label="Delete FCA"
          onConfirm={onDelete}
          warn={`Delete the “${fca.name}” FCA?`}
        >
          <Trash2 className="size-3.5" />
        </ConfirmButton>
      )}
    </li>
  );
}

/** One draggable row in the ROUTES panel (see #109 — order is per-viewer, via `usePersistedOrder`). */
function RouteRow({
  r,
  selected,
  canEditRoute,
  canDeleteRoute,
  labeled,
  hidden,
  onSelect,
  onToggleFixes,
  onToggleVisibility,
  onEdit,
  onDelete,
}: {
  r: MapRoute;
  selected: boolean;
  canEditRoute: boolean;
  canDeleteRoute: boolean;
  labeled: boolean;
  hidden: boolean;
  onSelect: () => void;
  onToggleFixes: () => void;
  onToggleVisibility: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: r.id,
  });
  const style = { transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.5 : 1 };
  return (
    <li
      ref={setNodeRef}
      style={style}
      className={
        "flex items-center gap-2 rounded px-2 py-1.5 text-sm " +
        (selected ? "bg-accent/40 " : "") +
        (hidden ? "opacity-50" : "")
      }
    >
      <button
        type="button"
        className="cursor-grab text-muted-foreground/60 hover:text-foreground"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-3.5" />
      </button>
      <span className="size-3 shrink-0 rounded-full" style={{ background: r.color, border: `2px solid ${r.color}` }} />
      <button type="button" title={r.route} onClick={onSelect} className="flex-1 truncate text-left font-mono">
        {r.name}
        {r.unresolved.length > 0 && (
          <span className="ml-1.5 text-xs text-amber-500" title={`Unresolved: ${r.unresolved.join(" ")}`}>
            ⚠{r.unresolved.length}
          </span>
        )}
      </button>
      <button
        type="button"
        title={hidden ? "Show route" : "Hide route"}
        onClick={onToggleVisibility}
        className="text-muted-foreground hover:text-foreground"
      >
        {hidden ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
      </button>
      <button
        type="button"
        title={labeled ? "Hide fix names" : "Show fix names"}
        onClick={onToggleFixes}
        className={"transition-colors " + (labeled ? "text-primary" : "text-muted-foreground hover:text-foreground")}
      >
        <Tag className="size-3.5" />
      </button>
      {canEditRoute && (
        <button type="button" title="Edit" onClick={onEdit} className="text-muted-foreground hover:text-foreground">
          <Pencil className="size-3.5" />
        </button>
      )}
      {canDeleteRoute && (
        <ConfirmButton
          size="icon"
          className="size-7"
          title="Delete"
          aria-label="Delete route"
          onConfirm={onDelete}
          warn={`Delete the “${r.name}” route?`}
        >
          <Trash2 className="size-3.5" />
        </ConfirmButton>
      )}
    </li>
  );
}

/**
 * The Flow Constrained Area map on the shared deck.gl TrafficMap. Read-only for now (the dashboard
 * widget + public advisories); the FCA builder's drawing/editing lands in the next phase. Reuses the
 * live/historical hooks so the embedded dashboard widget replays at the scrubber instant.
 */
export function FcaMapView({
  readOnly = false,
  overview = false,
  eventId,
  initialFlight,
  embedded = false,
  persistKey,
}: {
  readOnly?: boolean;
  /** ARTCC overview: overlay every active FCA in the selected ARTCC — matched traffic (tinted +
   *  numbered per FCA) and routes — at once, rather than one FCA at a time. */
  overview?: boolean;
  /** Event builder: scope every FCA (list + create/edit/delete) to this event, gated on events.plan.*
   *  instead of flow.fca.*. Event FCAs stay off the live maps until published. */
  eventId?: number;
  initialFlight?: string;
  embedded?: boolean;
  /** Stable key for remembering this map instance's pan/zoom (gated by the map.persistView setting). */
  persistKey?: string;
}) {
  const { data: me } = useMe();
  const toast = useToast();
  const { resolvedTheme } = useTheme();
  const eventMode = eventId != null;
  const canRead = readOnly || hasPermission(me, eventMode ? "events.plan.read" : "flow.fca.read");
  const canEdit = !readOnly && hasPermission(me, eventMode ? "events.plan.update" : "flow.fca.update");
  const canDelete =
    !readOnly && hasPermission(me, eventMode ? "events.plan.update" : "flow.fca.delete");
  // Routes are shared (not event-scoped); the event builder doesn't edit them.
  const canEditRoute = !readOnly && !eventMode && hasPermission(me, "flow.route.update");
  const canDeleteRoute = !readOnly && !eventMode && hasPermission(me, "flow.route.delete");

  const fcas = useFcas(eventId);
  const traffic = useTraffic();
  const routes = useRoutes();
  const createFca = useCreateFca(eventId);
  const updateFca = useUpdateFca(eventId);
  const deleteFca = useDeleteFca(eventId);
  const createRoute = useCreateRoute();
  const updateRoute = useUpdateRoute();
  const deleteRoute = useDeleteRoute();
  const routeIds = useMemo(() => (routes.data ?? []).map((r) => r.id), [routes.data]);
  const [routeOrder, setRouteOrder] = usePersistedOrder("fca.routeOrder", routeIds);
  const orderedRoutes = useMemo(() => {
    const byId = new Map((routes.data ?? []).map((r) => [r.id, r]));
    return routeOrder.map((id) => byId.get(id)).filter((r): r is NonNullable<typeof r> => !!r);
  }, [routes.data, routeOrder]);
  const routeDragSensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));
  function onRouteDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    const from = routeOrder.indexOf(String(active.id));
    const to = routeOrder.indexOf(String(over.id));
    if (from < 0 || to < 0) return;
    setRouteOrder(arrayMove(routeOrder, from, to));
  }

  // FCA draft (drawn on the map) + route form (a filed-route string).
  const [draft, setDraft] = useState<Draft | null>(null);
  const [phase, setPhase] = useState<Phase>("draw");
  const [routeForm, setRouteForm] = useState<RouteForm | null>(null);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedRouteId, setSelectedRouteId] = useState<string | null>(null);
  const [labeledRoutes, setLabeledRoutes] = useState<Set<string>>(new Set());
  // Display-only, per-viewer route visibility (see #108) — never touches the saved route data.
  const [hiddenRoutes, setHiddenRoutes] = useState<Set<string>>(() => {
    try {
      const raw = localStorage.getItem("fca.hiddenRoutes");
      const v: unknown = raw ? JSON.parse(raw) : [];
      return new Set(Array.isArray(v) && v.every((x) => typeof x === "string") ? v : []);
    } catch {
      return new Set();
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("fca.hiddenRoutes", JSON.stringify([...hiddenRoutes]));
    } catch {
      /* non-fatal */
    }
  }, [hiddenRoutes]);
  const [routeCallsign, setRouteCallsign] = useState<string | null>(null);
  const [mobileList, setMobileList] = useState(false);
  const [filter, setFilter] = useState("");
  const [artccFilter, setArtccFilter] = useState("");

  const [planeIcons, setPlaneIcons] = useState(() => {
    try {
      return localStorage.getItem("fca.planeIcons") === "1";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("fca.planeIcons", planeIcons ? "1" : "0");
    } catch {
      /* non-fatal */
    }
  }, [planeIcons]);

  const [showAtc, setShowAtc] = useState(() => {
    try {
      return localStorage.getItem("fca.atc") === "1";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("fca.atc", showAtc ? "1" : "0");
    } catch {
      /* non-fatal */
    }
  }, [showAtc]);
  const atc = useAtc(showAtc);

  const debug = useSetting("debug.enabled", false).value;
  const predictionScrubberSetting = useSetting("debug.predictionScrubber", false).value;
  const predictionScrubberEnabled = debug && predictionScrubberSetting;
  const [offsetSec, setOffsetSec] = useState(0);
  // Debounce the fetched offset so a fast drag doesn't fire a fresh backend projection (real
  // per-aircraft route resolution, run under spawn_blocking) on every slider tick — the slider
  // itself still tracks `offsetSec` immediately for a responsive readout.
  const [committedOffsetSec, setCommittedOffsetSec] = useState(0);
  useEffect(() => {
    const id = window.setTimeout(() => setCommittedOffsetSec(offsetSec), 150);
    return () => window.clearTimeout(id);
  }, [offsetSec]);
  const scrubberActive = predictionScrubberEnabled && committedOffsetSec > 0;
  const predictedTraffic = usePredictedTraffic(committedOffsetSec);
  const fcaTraffic = useFcaTraffic(draft ? null : selectedId, debug);
  const counts = useFcaCounts();
  const aircraftRoute = useAircraftRoute(routeCallsign);
  const dataStatus = useDataStatus();
  const cycleAge = dataStatus.data ? cycleAgeDays(dataStatus.data.nav_cycle) : null;
  const navStale = cycleAge != null && cycleAge > 35;

  const { value: persistView } = useSetting("map.persistView", true);
  const camera = useMapCamera(FCA_INITIAL, { persistKey, persist: persistView && !!persistKey });

  const selectedFca = useMemo(
    () => fcas.data?.find((f) => f.id === selectedId) ?? null,
    [fcas.data, selectedId],
  );

  // Locate a callsign in the live traffic, fly to it, and plot its route.
  const focusFlight = (callsign: string) => {
    const cs = callsign.trim().toUpperCase();
    if (!cs) return;
    const ac = traffic.data?.find((a) => a.callsign.toUpperCase() === cs);
    if (ac) {
      camera.flyTo({ longitude: ac.lon, latitude: ac.lat, zoom: Math.max(camera.viewState.zoom ?? 4, 6) });
      setRouteCallsign(cs);
    } else {
      toast.warning(`${cs} isn’t in the live traffic right now`);
    }
  };

  // Deep link from the pilot page (?flight=CALLSIGN): locate it once traffic loads.
  const deepLinked = useRef(false);
  useEffect(() => {
    if (!initialFlight || deepLinked.current || !traffic.data) return;
    deepLinked.current = true;
    focusFlight(initialFlight);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialFlight, traffic.data]);

  const selectFca = (id: string) => {
    setMobileList(false);
    const fca = fcas.data?.find((f) => f.id === id);
    // Overview drives the right panel off the ARTCC selection — a sidebar click just re-centers the
    // map on that FCA, it doesn't open a single-FCA detail.
    if (overview) {
      if (fca && fca.points.length >= 2) {
        camera.fitBounds(toDeckPath(fca.points as LatLng[]), { padding: 80, maxZoom: 9 });
      }
      return;
    }
    const next = selectedId === id ? null : id;
    setSelectedId(next);
    if (next && fca && fca.points.length >= 2) {
      camera.fitBounds(toDeckPath(fca.points as LatLng[]), { padding: 80, maxZoom: 9 });
    }
  };

  const toggleFixes = (id: string) =>
    setLabeledRoutes((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const toggleRouteVisibility = (id: string) =>
    setHiddenRoutes((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const allRoutesHidden =
    !!routes.data && routes.data.length > 0 && routes.data.every((r) => hiddenRoutes.has(r.id));
  const toggleAllRoutesVisibility = () =>
    setHiddenRoutes(allRoutesHidden ? new Set() : new Set(routes.data?.map((r) => r.id)));

  // --- FCA drawing / editing ---
  const drawing = !!draft && phase === "draw";
  const editing = !!draft && phase === "edit";
  const cancel = () => setDraft(null);
  const finishLine = () => setDraft((d) => (d && d.points.length >= 2 ? (setPhase("edit"), d) : d));

  const startNew = () => {
    setSelectedId(null);
    setRouteCallsign(null);
    setDraft(blankDraft(fcas.data?.length ?? 0));
    setPhase("draw");
  };
  const startEdit = (fca: Fca) => {
    setSelectedId(null);
    setRouteCallsign(null);
    setDraft(draftFrom(fca));
    setPhase("edit");
  };
  const addVertex = ([lon, lat]: [number, number]) =>
    setDraft((d) => {
      if (!d) return d;
      const p: LatLng = [lat, lon];
      const last = d.points[d.points.length - 1];
      if (last && haversine(last, p) < 0.4) return d; // dedupe double-click
      return { ...d, points: [...d.points, p] };
    });
  const moveVertex = (i: number, [lon, lat]: [number, number]) =>
    setDraft((d) => (d ? { ...d, points: d.points.map((p, idx) => (idx === i ? [lat, lon] : p)) } : d));

  const save = () => {
    if (!draft || draft.points.length < 2) return;
    const body: UpsertFca = {
      name: draft.name.trim() || "FCA",
      color: draft.color,
      artcc: draft.artcc.trim().toUpperCase(),
      points: normPoints(draft.points),
      dests: parseList(draft.dests),
      origins: parseList(draft.origins),
      fixes: parseList(draft.fixes),
      scope: parseList(draft.scope),
      min_fl: parseFl(draft.minFl),
      max_fl: parseFl(draft.maxFl),
      mode: draft.mode,
      rate: draft.rate,
      mit: draft.mit,
    };
    if (draft.id) updateFca.mutate({ id: draft.id, body }, { onSuccess: cancel });
    else createFca.mutate(body, { onSuccess: cancel });
  };
  const toggleEnabled = (fca: Fca) =>
    updateFca.mutate({ id: fca.id, body: { ...toUpsert(fca), enabled: !fca.enabled } });

  const startNewRoute = () => {
    setSelectedRouteId(null);
    setRouteForm(blankRouteForm(routes.data?.length ?? 0));
  };
  const startEditRoute = (r: MapRoute) => {
    setSelectedRouteId(null);
    setRouteForm(routeFormFrom(r));
  };
  const saveRoute = () => {
    if (!routeForm || !routeForm.name.trim() || !routeForm.route.trim()) return;
    const body: UpsertRoute = {
      name: routeForm.name.trim(),
      color: routeForm.color,
      route: routeForm.route.trim().toUpperCase(),
      dep: routeForm.dep.trim().toUpperCase(),
      arr: routeForm.arr.trim().toUpperCase(),
    };
    const done = () => setRouteForm(null);
    if (routeForm.id) updateRoute.mutate({ id: routeForm.id, body }, { onSuccess: done });
    else createRoute.mutate(body, { onSuccess: done });
  };

  // Keyboard while drawing: Enter finish · Esc cancel · Backspace undo.
  useEffect(() => {
    if (!drawing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        finishLine();
      } else if (e.key === "Escape") {
        setDraft(null);
      } else if (e.key === "Backspace") {
        e.preventDefault();
        setDraft((d) => (d ? { ...d, points: d.points.slice(0, -1) } : d));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drawing]);

  const draftLine = useMemo(
    () => (draft ? { color: draft.color, points: draft.points } : null),
    [draft],
  );

  // ARTCC overview: every enabled FCA — nationwide when "ALL", or scoped to the selected ARTCC. All
  // their matched traffic draws on the map at once, and the right panel stacks their strips.
  const overviewFcas = useMemo(
    () =>
      overview
        ? (fcas.data ?? []).filter(
            (f) => f.enabled && f.points.length >= 2 && (!artccFilter || f.artcc === artccFilter),
          )
        : [],
    [overview, artccFilter, fcas.data],
  );
  const overviewActive = overview;
  const overviewTraffic = useFcaTrafficMany(overviewFcas.map((f) => f.id));
  // A primitive signature so the group/callsign memos recompute only when the data (or set) changes.
  const overviewSig = overviewTraffic
    .map((r, i) => `${overviewFcas[i]?.id}:${overviewFcas[i]?.color}:${r.dataUpdatedAt}`)
    .join("|");
  const matchedGroups = useMemo(
    () =>
      overviewFcas.map((f, i) => ({
        id: f.id,
        color: f.color,
        flights: (overviewTraffic[i]?.data ?? []) as MatchedFlight[],
      })),
    // overviewSig captures the ids, colors, and per-FCA data freshness that actually affect the output.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [overviewSig],
  );
  // Same data, keyed by FCA, for the right-hand strips panel (needs the full FcaFlight rows).
  const overviewGroups = useMemo<OverviewGroup[]>(
    () => overviewFcas.map((f, i) => ({ fca: f, flights: overviewTraffic[i]?.data })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [overviewSig],
  );

  // Matched (crossing) traffic — for the selected FCA, or every overview FCA — is drawn separately
  // (tinted + numbered); exclude those callsigns from the plain traffic layer.
  const matchedCallsigns = useMemo(() => {
    const set = new Set<string>();
    if (overview) {
      for (const g of matchedGroups) for (const f of g.flights) set.add(f.callsign);
    } else if (selectedFca) {
      for (const f of fcaTraffic.data ?? []) set.add(f.callsign);
    }
    return set;
  }, [overview, matchedGroups, selectedFca, fcaTraffic.data]);
  // #226: with the prediction scrubber dragged forward, swap the map's traffic source for the
  // projected positions — everything downstream (overview filtering, matched-callsign exclusion,
  // the NormAircraft mapping) is unchanged, since projected traffic is the exact same shape.
  const trafficSource = scrubberActive ? (predictedTraffic.data ?? traffic.data) : traffic.data;
  const aircraft = useMemo<NormAircraft[]>(() => {
    // In overview mode only FCA-crossing traffic is shown (drawn as the tinted/numbered matched
    // groups); the rest of the network is noise here — except the flight the user deep-linked to
    // (`?flight=`) or clicked, which is always drawn so it's visible even without a crossing.
    const base = overviewActive
      ? routeCallsign
        ? (trafficSource ?? []).filter((a) => a.callsign.toUpperCase() === routeCallsign)
        : []
      : (trafficSource ?? []);
    return base
      .filter((a) => !matchedCallsigns.has(a.callsign))
      .map((a) => ({
        id: a.callsign,
        callsign: a.callsign,
        actype: a.actype,
        dep: a.dep,
        arr: a.arr,
        lat: a.lat,
        lon: a.lon,
        alt: a.alt,
        gs: a.gs,
        heading: a.heading,
        star: a.star,
        wake: a.wake,
        flightRules: a.flight_rules,
        filedAlt: a.filed_alt,
      }));
  }, [overviewActive, routeCallsign, trafficSource, matchedCallsigns]);

  const mapFcas = useMemo<MapFca[]>(
    () =>
      (fcas.data ?? []).map((f) => ({
        id: f.id,
        name: f.name,
        color: f.color,
        enabled: f.enabled,
        points: f.points as LatLng[],
      })),
    [fcas.data],
  );

  const filedRoute = useMemo(
    () =>
      aircraftRoute.data
        ? { path: toDeckPath(aircraftRoute.data.points as LatLng[]), waypoints: aircraftRoute.data.waypoints }
        : null,
    [aircraftRoute.data],
  );

  // Memoized so the named-routes map layers only rebuild (re-tessellate/re-upload) when the route
  // set or hidden-set actually changes, not on every unrelated FcaMapView re-render.
  const visibleRoutes = useMemo(
    () => (routes.data as NamedRoute[] | undefined)?.filter((r) => !hiddenRoutes.has(r.id)),
    [routes.data, hiddenRoutes],
  );

  // Custom order applies over the FULL live FCA set, never the filtered view — a drag performed
  // while `filter`/`artccFilter` narrows the list must not discard position info for FCAs
  // currently hidden by that filter (see #109 QA: feeding this the filtered list let a filtered
  // drag silently overwrite the stored order with just the visible subset).
  const allFcaIds = useMemo(() => (fcas.data ?? []).map((f) => f.id), [fcas.data]);
  const [fcaOrder, setFcaOrder] = usePersistedOrder("fca.fcaOrder", allFcaIds);
  const orderedAllFcas = useMemo(() => {
    const byId = new Map((fcas.data ?? []).map((f) => [f.id, f]));
    return fcaOrder.map((id) => byId.get(id)).filter((f): f is NonNullable<typeof f> => !!f);
  }, [fcas.data, fcaOrder]);
  // Sidebar FCA list — the ordered set above, filtered by name/fix + ARTCC for display.
  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return orderedAllFcas.filter((f) => {
      if (artccFilter && f.artcc !== artccFilter) return false;
      if (!q) return true;
      return (
        f.name.toLowerCase().includes(q) ||
        f.fixes.some((x) => x.toLowerCase().includes(q)) ||
        f.artcc.toLowerCase().includes(q)
      );
    });
  }, [orderedAllFcas, filter, artccFilter]);
  const fcaDragSensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));
  function onFcaDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    // Indices in the full order, not the (possibly filtered) visible list — dragging within a
    // filtered view still repositions the dragged item relative to the drop target in the
    // complete list, so nothing outside the current filter gets dropped from `order`.
    const from = fcaOrder.indexOf(String(active.id));
    const to = fcaOrder.indexOf(String(over.id));
    if (from < 0 || to < 0) return;
    setFcaOrder(arrayMove(fcaOrder, from, to));
  }
  const artccOptions = useMemo(
    () => [...new Set((fcas.data ?? []).map((f) => f.artcc).filter(Boolean))].sort(),
    [fcas.data],
  );
  // Frame the map to the ARTCC selection in overview mode: a specific ARTCC's extent, or the whole
  // CONUS for "ALL" (which stacks every FCA nationwide).
  useEffect(() => {
    if (!overview) return;
    if (!artccFilter) {
      camera.home();
      return;
    }
    const feature = facilityFeature(artccFilter);
    const pts = feature ? facilityPoints(feature) : [];
    if (pts.length) camera.fitBounds(pts, { padding: 40, maxZoom: 7 });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [overview, artccFilter]);

  if (!canRead) {
    return (
      <div
        className={`flex items-center justify-center text-sm text-muted-foreground ${
          embedded ? "h-full" : "h-[calc(100vh-3.5rem)]"
        }`}
      >
        You don&apos;t have flow access.
      </div>
    );
  }

  return (
    // `isolate` keeps the map's high internal z-indexes in their own stacking context so they don't
    // paint over app chrome (nav dropdowns, toasts, dialogs), which portal to the body above it.
    <div className={`relative isolate flex ${embedded ? "h-full" : "h-[calc(100vh-3.5rem)]"}`}>
      {!embedded && mobileList && (
        <div className="absolute inset-0 z-[650] bg-black/40 md:hidden" onClick={() => setMobileList(false)} />
      )}

      {!embedded && (
        <aside
          className={
            "flex w-80 shrink-0 flex-col border-r bg-background max-md:absolute max-md:inset-y-0 max-md:left-0 max-md:z-[700] max-md:w-[85%] max-md:max-w-xs max-md:shadow-2xl max-md:transition-transform " +
            (mobileList ? "max-md:translate-x-0" : "max-md:-translate-x-full")
          }
        >
          <div className="flex items-center gap-2 border-b px-4 py-3">
            {eventMode && (
              <Link
                to="/planning/events/$eventId"
                params={{ eventId: String(eventId) }}
                title="Back to the event"
                className="text-muted-foreground transition-colors hover:text-foreground"
              >
                <ArrowLeft className="size-4" />
              </Link>
            )}
            <span className="text-sm font-semibold uppercase tracking-wide">
              {eventMode ? "Event FCAs" : "Flow Constrained Areas"}
            </span>
            <button
              type="button"
              aria-label="Close list"
              onClick={() => setMobileList(false)}
              className="ml-auto text-muted-foreground hover:text-foreground md:hidden"
            >
              <X className="size-4" />
            </button>
          </div>

          {editing && draft ? (
            <DraftEditor
              draft={draft}
              onChange={setDraft}
              onSave={save}
              onRedraw={() => {
                setDraft((d) => (d ? { ...d, points: [] } : d));
                setPhase("draw");
              }}
              onCancel={cancel}
              saving={createFca.isPending || updateFca.isPending}
            />
          ) : (
            <>
              {canEdit && (
                <div className="border-b p-3">
                  {drawing ? (
                    <Button variant="secondary" className="w-full" onClick={cancel}>
                      Cancel drawing
                    </Button>
                  ) : (
                    <Button className="w-full" onClick={startNew}>
                      <Plus />
                      New FCA
                    </Button>
                  )}
                </div>
              )}
              <div className="flex flex-col gap-2 border-b p-3">
                <select
                  value={artccFilter}
                  onChange={(e) => setArtccFilter(e.target.value)}
                  className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <option value="">ALL ARTCCs</option>
                  {artccOptions.map((a) => (
                    <option key={a} value={a}>
                      {a}
                    </option>
                  ))}
                </select>
                <Input placeholder="filter — name or fix…" value={filter} onChange={(e) => setFilter(e.target.value)} />
              </div>

              <div className="flex-1 overflow-y-auto">
                {!fcas.data ? (
                  <p className="p-4 text-sm text-muted-foreground">Loading…</p>
                ) : shown.length === 0 ? (
                  <p className="p-4 text-sm text-muted-foreground">
                    No FCAs.{canEdit && " Draw one with “New FCA”."}
                  </p>
                ) : (
                  <DndContext
                    sensors={fcaDragSensors}
                    collisionDetection={closestCenter}
                    onDragEnd={onFcaDragEnd}
                  >
                    <SortableContext items={shown.map((f) => f.id)} strategy={verticalListSortingStrategy}>
                      <ul>
                        {shown.map((fca) => (
                          <FcaRow
                            key={fca.id}
                            fca={fca}
                            selected={fca.id === selectedId}
                            count={counts.data?.[fca.id] ?? 0}
                            canEdit={canEdit}
                            canDelete={canDelete}
                            onSelect={() => selectFca(fca.id)}
                            onToggleEnabled={() => toggleEnabled(fca)}
                            onEdit={() => startEdit(fca)}
                            onDelete={() => deleteFca.mutate(fca.id)}
                          />
                        ))}
                      </ul>
                    </SortableContext>
                  </DndContext>
                )}
              </div>

              <div className="flex flex-col border-t">
                {routeForm ? (
                  <RouteEditor
                    form={routeForm}
                    onChange={setRouteForm}
                    onSave={saveRoute}
                    onCancel={() => setRouteForm(null)}
                    saving={createRoute.isPending || updateRoute.isPending}
                  />
                ) : (
                  <>
                    <div className="flex items-center justify-between px-3 pt-3">
                      <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Routes</span>
                      <div className="flex items-center gap-3">
                        {routes.data && routes.data.length > 0 && (
                          <button
                            type="button"
                            title={allRoutesHidden ? "Show all routes" : "Hide all routes"}
                            onClick={toggleAllRoutesVisibility}
                            className="text-muted-foreground hover:text-foreground"
                          >
                            {allRoutesHidden ? (
                              <EyeOff className="size-3.5" />
                            ) : (
                              <Eye className="size-3.5" />
                            )}
                          </button>
                        )}
                        {canEditRoute && (
                          <button
                            type="button"
                            onClick={startNewRoute}
                            className="flex items-center gap-1 text-xs text-primary hover:underline"
                          >
                            <Plus className="size-3.5" />
                            New route
                          </button>
                        )}
                      </div>
                    </div>
                    {routes.data && routes.data.length > 0 ? (
                      <DndContext
                        sensors={routeDragSensors}
                        collisionDetection={closestCenter}
                        onDragEnd={onRouteDragEnd}
                      >
                        <SortableContext items={routeOrder} strategy={verticalListSortingStrategy}>
                          <ul className="max-h-48 overflow-y-auto p-1">
                            {orderedRoutes.map((r) => (
                              <RouteRow
                                key={r.id}
                                r={r}
                                selected={r.id === selectedRouteId}
                                canEditRoute={canEditRoute}
                                canDeleteRoute={canDeleteRoute}
                                labeled={labeledRoutes.has(r.id)}
                                hidden={hiddenRoutes.has(r.id)}
                                onSelect={() => setSelectedRouteId((cur) => (cur === r.id ? null : r.id))}
                                onToggleFixes={() => toggleFixes(r.id)}
                                onToggleVisibility={() => toggleRouteVisibility(r.id)}
                                onEdit={() => startEditRoute(r)}
                                onDelete={() => deleteRoute.mutate(r.id)}
                              />
                            ))}
                          </ul>
                        </SortableContext>
                      </DndContext>
                    ) : (
                      <p className="px-3 py-3 text-xs text-muted-foreground">
                        No routes yet.{canEditRoute && " Add one with “New route”."}
                      </p>
                    )}
                  </>
                )}
              </div>
            </>
          )}

          <div className="border-t px-4 py-2 text-xs">
            {dataStatus.data ? (
              <div className="min-w-0">
                <div className="flex items-center gap-1.5 font-mono">
                  <span className={`size-1.5 shrink-0 rounded-full ${navStale ? "bg-amber-500" : "bg-emerald-500"}`} />
                  <span>NASR {dataStatus.data.nav_cycle}</span>
                  {cycleAge != null && (
                    <span className={navStale ? "text-amber-500" : "text-muted-foreground"}> · {cycleAge}d</span>
                  )}
                </div>
                <div className="truncate text-muted-foreground">
                  {dataStatus.data.winds_stations} winds · {traffic.data?.length ?? 0} traffic
                </div>
              </div>
            ) : (
              <span className="text-muted-foreground">
                {traffic.data ? `traffic ${traffic.data.length}` : "traffic…"}
              </span>
            )}
          </div>
          <CoveragePanel />
        </aside>
      )}

      <TrafficMap
        className="relative isolate flex-1 overflow-hidden"
        baseCursor="crosshair"
        camera={camera}
        aircraft={aircraft}
        aircraftStyle={planeIcons ? "silhouette" : "triangle"}
        selectedAircraftId={routeCallsign}
        getAircraftColor={(a) =>
          a.id.toUpperCase() === routeCallsign ? HIGHLIGHT : aircraftColor(resolvedTheme)
        }
        boundaries={BOUNDARIES}
        fcas={mapFcas}
        selectedFcaId={selectedId}
        atc={showAtc ? (atc.data as AtcData | undefined) ?? null : null}
        matched={!overview && selectedFca ? (fcaTraffic.data as MatchedFlight[] | undefined) : undefined}
        matchedColor={overview ? undefined : selectedFca?.color}
        matchedGroups={overview ? matchedGroups : undefined}
        namedRoutes={visibleRoutes}
        selectedRouteId={selectedRouteId}
        labeledRouteIds={labeledRoutes}
        filedRoute={filedRoute}
        draft={draftLine}
        drawMode={draft ? phase : null}
        onAddVertex={addVertex}
        onMoveVertex={moveVertex}
        onFinishDraft={finishLine}
        onAircraftClick={(cs) => setRouteCallsign((cur) => (cur === cs ? null : cs))}
        onMatchedClick={(cs) => setRouteCallsign((cur) => (cur === cs ? null : cs))}
        onFcaClick={selectFca}
      >
        {/* Map controls */}
        <div className="absolute left-3 top-3 z-[500] flex max-w-[calc(100%-1.5rem)] flex-wrap items-center gap-2">
          {!embedded && (
            <button
              type="button"
              onClick={() => setMobileList(true)}
              title="Show the FCA list"
              className="flex items-center gap-1.5 rounded-lg border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors hover:bg-muted md:hidden"
            >
              <Menu className="size-3.5 text-muted-foreground" />
              List
            </button>
          )}
          <button
            type="button"
            onClick={() => camera.home()}
            title="Center on the United States"
            className="flex items-center gap-1.5 rounded-lg border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors hover:bg-muted"
          >
            <Home className="size-3.5 text-muted-foreground" />
            Home
          </button>
          <button
            type="button"
            onClick={() => setPlaneIcons((v) => !v)}
            title={planeIcons ? "Live traffic: aircraft-type silhouettes" : "Live traffic: plain triangles"}
            className="flex items-center gap-1.5 rounded-lg border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors hover:bg-muted"
          >
            <Plane className={`size-3.5 ${planeIcons ? "text-primary" : "text-muted-foreground"}`} />
            {planeIcons ? "Aircraft icons" : "Triangles"}
          </button>
          <button
            type="button"
            onClick={() => setShowAtc((v) => !v)}
            title="Toggle online ATC (positions, approach & center areas)"
            aria-pressed={showAtc}
            className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors ${
              showAtc ? "border-primary/60 bg-primary/15 text-primary" : "bg-background/95 hover:bg-muted"
            }`}
          >
            <RadioTower className={`size-3.5 ${showAtc ? "text-primary" : "text-muted-foreground"}`} />
            ATC
          </button>
          <FlightSearch
            aircraft={traffic.data ?? []}
            onSelect={focusFlight}
            placeholder="find flight…"
            variant="overlay"
          />
        </div>

        {predictionScrubberEnabled && (
          <div className="absolute inset-x-0 bottom-3 z-[500] flex justify-center px-3">
            <div className="flex w-full max-w-xl items-center gap-3 rounded-lg border bg-background/95 px-3 py-2 text-xs shadow-lg backdrop-blur">
              <span className="shrink-0 font-medium text-muted-foreground">
                {offsetSec === 0 ? "Live" : `T+${Math.round(offsetSec / 60)}m`}
              </span>
              <input
                type="range"
                min={0}
                max={MAX_PROJECTION_SEC}
                step={30}
                value={offsetSec}
                onChange={(e) => setOffsetSec(Number(e.target.value))}
                className="flex-1"
                aria-label="Prediction scrubber — minutes ahead"
              />
              <button
                type="button"
                onClick={() => setOffsetSec(0)}
                disabled={offsetSec === 0}
                className="shrink-0 rounded-md border px-2 py-1 font-medium transition-colors hover:bg-muted disabled:opacity-50"
              >
                Reset
              </button>
            </div>
          </div>
        )}

        {navStale && (
          <div className="pointer-events-none absolute inset-x-0 top-3 z-[500] flex justify-center">
            <div className="pointer-events-auto flex items-center gap-2 rounded-lg border border-amber-500/50 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 shadow-lg backdrop-blur dark:text-amber-200">
              <span>
                NASR data is {cycleAge} days old ({dataStatus.data?.nav_cycle}).
              </span>
            </div>
          </div>
        )}

        {drawing && draft && (
          <div className="pointer-events-none absolute inset-x-0 bottom-6 z-[500] flex justify-center">
            <div className="pointer-events-auto flex items-center gap-2.5 rounded-lg border border-primary/60 bg-background/95 px-5 py-3 text-sm shadow-lg backdrop-blur">
              <span className="font-medium">Click to add points</span>
              <Kbd>⌫</Kbd>
              <span className="text-muted-foreground">undo</span>
              <Kbd>dbl-click</Kbd>
              <span className="text-muted-foreground">or</span>
              <Kbd>↵</Kbd>
              <span className="text-muted-foreground">finish</span>
              <Kbd>esc</Kbd>
              <span className="text-muted-foreground">cancel</span>
              <span className="ml-1 border-l pl-3 font-mono text-xs text-muted-foreground">
                {draft.points.length} pts
              </span>
            </div>
          </div>
        )}

        {routeCallsign && aircraftRoute.data && (
          <RoutePopup
            route={aircraftRoute.data}
            fca={selectedFca}
            match={fcaTraffic.data?.find((f) => f.callsign === aircraftRoute.data!.callsign)}
            onClose={() => setRouteCallsign(null)}
          />
        )}
      </TrafficMap>

      {overviewActive && !draft ? (
        <FcaOverviewPanel
          artcc={artccFilter || "ALL"}
          groups={overviewGroups}
          onFocusFlight={focusFlight}
        />
      ) : (
        selectedFca &&
        !draft && (
          <FcaDetail fca={selectedFca} flights={fcaTraffic.data} canEdit={canEdit} onClose={() => setSelectedId(null)} />
        )
      )}
    </div>
  );
}
