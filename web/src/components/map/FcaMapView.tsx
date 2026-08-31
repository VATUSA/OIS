import {useEffect, useMemo, useRef, useState} from "react";
import {Button, ConfirmButton, Input, Switch, useToast} from "@ois/ui";
import {Home, Menu, Pencil, Plane, Plus, RadioTower, Tag, Trash2, X} from "lucide-react";

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
import {useSetting} from "@/lib/settings";
import {FlightSearch} from "@/components/flight-search";
import {FcaDetail} from "@/pages/fca/detail";
import {FcaOverviewPanel, type OverviewGroup} from "@/pages/fca/overview-panel";
import {facilityFeature, facilityPoints} from "@/lib/facility-map/boundary";
import boundariesGeo from "@/assets/artcc-boundaries.json";

import {TrafficMap} from "./TrafficMap";
import {useMapCamera} from "./hooks/useMapCamera";
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

/**
 * The Flow Constrained Area map on the shared deck.gl TrafficMap. Read-only for now (the dashboard
 * widget + public advisories); the FCA builder's drawing/editing lands in the next phase. Reuses the
 * live/historical hooks so the embedded dashboard widget replays at the scrubber instant.
 */
export function FcaMapView({
  readOnly = false,
  overview = false,
  initialFlight,
  embedded = false,
  persistKey,
}: {
  readOnly?: boolean;
  /** ARTCC overview: overlay every active FCA in the selected ARTCC — matched traffic (tinted +
   *  numbered per FCA) and routes — at once, rather than one FCA at a time. */
  overview?: boolean;
  initialFlight?: string;
  embedded?: boolean;
  /** Stable key for remembering this map instance's pan/zoom (gated by the map.persistView setting). */
  persistKey?: string;
}) {
  const { data: me } = useMe();
  const toast = useToast();
  const canRead = readOnly || hasPermission(me, "flow.fca.read");
  const canEdit = !readOnly && hasPermission(me, "flow.fca.update");
  const canDelete = !readOnly && hasPermission(me, "flow.fca.delete");
  const canEditRoute = !readOnly && hasPermission(me, "flow.route.update");
  const canDeleteRoute = !readOnly && hasPermission(me, "flow.route.delete");

  const fcas = useFcas();
  const traffic = useTraffic();
  const routes = useRoutes();
  const createFca = useCreateFca();
  const updateFca = useUpdateFca();
  const deleteFca = useDeleteFca();
  const createRoute = useCreateRoute();
  const updateRoute = useUpdateRoute();
  const deleteRoute = useDeleteRoute();

  // FCA draft (drawn on the map) + route form (a filed-route string).
  const [draft, setDraft] = useState<Draft | null>(null);
  const [phase, setPhase] = useState<Phase>("draw");
  const [routeForm, setRouteForm] = useState<RouteForm | null>(null);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedRouteId, setSelectedRouteId] = useState<string | null>(null);
  const [labeledRoutes, setLabeledRoutes] = useState<Set<string>>(new Set());
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

  const fcaTraffic = useFcaTraffic(draft ? null : selectedId);
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

  // ARTCC overview: every enabled FCA in the SELECTED ARTCC. All their matched traffic draws on the
  // map at once, and the right panel stacks their strips. Requires a specific ARTCC (not "ALL").
  const overviewFcas = useMemo(
    () =>
      overview && artccFilter
        ? (fcas.data ?? []).filter((f) => f.enabled && f.artcc === artccFilter && f.points.length >= 2)
        : [],
    [overview, artccFilter, fcas.data],
  );
  const overviewActive = overview && !!artccFilter;
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
  const aircraft = useMemo<NormAircraft[]>(
    () =>
      (traffic.data ?? [])
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
        })),
    [traffic.data, matchedCallsigns],
  );

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

  // Sidebar FCA list — filtered by name/fix + ARTCC.
  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return (fcas.data ?? []).filter((f) => {
      if (artccFilter && f.artcc !== artccFilter) return false;
      if (!q) return true;
      return (
        f.name.toLowerCase().includes(q) ||
        f.fixes.some((x) => x.toLowerCase().includes(q)) ||
        f.artcc.toLowerCase().includes(q)
      );
    });
  }, [fcas.data, filter, artccFilter]);
  const artccOptions = useMemo(
    () => [...new Set((fcas.data ?? []).map((f) => f.artcc).filter(Boolean))].sort(),
    [fcas.data],
  );
  // In overview mode, default to the first ARTCC that has FCAs so the map isn't blank on first load.
  useEffect(() => {
    if (overview && !artccFilter && artccOptions.length) setArtccFilter(artccOptions[0]);
  }, [overview, artccFilter, artccOptions]);
  // On selecting an ARTCC in overview mode, zoom the map out to that ARTCC's extent.
  useEffect(() => {
    if (!overview || !artccFilter) return;
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
          <div className="flex items-center justify-between border-b px-4 py-3">
            <span className="text-sm font-semibold uppercase tracking-wide">Flow Constrained Areas</span>
            <button
              type="button"
              aria-label="Close list"
              onClick={() => setMobileList(false)}
              className="text-muted-foreground hover:text-foreground md:hidden"
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
                  <ul>
                    {shown.map((fca) => (
                      <li
                        key={fca.id}
                        className={
                          "flex items-center gap-2 border-b px-3 py-2 text-sm " +
                          (fca.id === selectedId ? "bg-accent/40 " : "") +
                          (fca.enabled ? "" : "opacity-55")
                        }
                      >
                        <span
                          className="size-3 shrink-0 rounded-full"
                          style={{ background: fca.color }}
                          title={fca.enabled ? "Enabled" : "Disabled"}
                        />
                        <button
                          type="button"
                          onClick={() => selectFca(fca.id)}
                          className="flex-1 truncate text-left font-mono"
                        >
                          {fca.name}
                          {fca.artcc && <span className="ml-1.5 text-xs text-muted-foreground">{fca.artcc}</span>}
                        </button>
                        <span
                          className={
                            "shrink-0 rounded px-1.5 text-xs font-medium tabular-nums " +
                            ((counts.data?.[fca.id] ?? 0) > 0 ? "bg-primary/15 text-primary" : "text-muted-foreground/50")
                          }
                        >
                          {counts.data?.[fca.id] ?? 0}
                        </span>
                        {canEdit && (
                          <span
                            className="flex shrink-0 items-center"
                            title={fca.enabled ? "Enabled — click to disable" : "Disabled — click to enable"}
                          >
                            <Switch
                              checked={fca.enabled}
                              onCheckedChange={() => toggleEnabled(fca)}
                              aria-label={`${fca.enabled ? "Disable" : "Enable"} the ${fca.name} FCA`}
                              className="scale-[0.68]"
                            />
                          </span>
                        )}
                        {canEdit && (
                          <button
                            type="button"
                            title="Edit"
                            onClick={() => startEdit(fca)}
                            className="text-muted-foreground hover:text-foreground"
                          >
                            <Pencil className="size-3.5" />
                          </button>
                        )}
                        {canDelete && (
                          <ConfirmButton
                            size="icon"
                            className="size-7"
                            title="Delete"
                            aria-label="Delete FCA"
                            onConfirm={() => deleteFca.mutate(fca.id)}
                            warn={`Delete the “${fca.name}” FCA?`}
                          >
                            <Trash2 className="size-3.5" />
                          </ConfirmButton>
                        )}
                      </li>
                    ))}
                  </ul>
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
                    {routes.data && routes.data.length > 0 ? (
                      <ul className="max-h-48 overflow-y-auto p-1">
                        {routes.data.map((r) => (
                          <li
                            key={r.id}
                            className={
                              "flex items-center gap-2 rounded px-2 py-1.5 text-sm " +
                              (r.id === selectedRouteId ? "bg-accent/40" : "")
                            }
                          >
                            <span className="size-3 shrink-0 rounded-full" style={{ background: r.color, border: `2px solid ${r.color}` }} />
                            <button
                              type="button"
                              title={r.route}
                              onClick={() => setSelectedRouteId((cur) => (cur === r.id ? null : r.id))}
                              className="flex-1 truncate text-left font-mono"
                            >
                              {r.name}
                              {r.unresolved.length > 0 && (
                                <span className="ml-1.5 text-xs text-amber-500" title={`Unresolved: ${r.unresolved.join(" ")}`}>
                                  ⚠{r.unresolved.length}
                                </span>
                              )}
                            </button>
                            <button
                              type="button"
                              title={labeledRoutes.has(r.id) ? "Hide fix names" : "Show fix names"}
                              onClick={() => toggleFixes(r.id)}
                              className={
                                "transition-colors " +
                                (labeledRoutes.has(r.id) ? "text-primary" : "text-muted-foreground hover:text-foreground")
                              }
                            >
                              <Tag className="size-3.5" />
                            </button>
                            {canEditRoute && (
                              <button
                                type="button"
                                title="Edit"
                                onClick={() => startEditRoute(r)}
                                className="text-muted-foreground hover:text-foreground"
                              >
                                <Pencil className="size-3.5" />
                              </button>
                            )}
                            {canDeleteRoute && (
                              <ConfirmButton
                                size="icon"
                                className="size-7"
                                title="Delete"
                                aria-label="Delete route"
                                onConfirm={() => deleteRoute.mutate(r.id)}
                                warn={`Delete the “${r.name}” route?`}
                              >
                                <Trash2 className="size-3.5" />
                              </ConfirmButton>
                            )}
                          </li>
                        ))}
                      </ul>
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
        boundaries={BOUNDARIES}
        fcas={mapFcas}
        selectedFcaId={selectedId}
        atc={showAtc ? (atc.data as AtcData | undefined) ?? null : null}
        matched={!overview && selectedFca ? (fcaTraffic.data as MatchedFlight[] | undefined) : undefined}
        matchedColor={overview ? undefined : selectedFca?.color}
        matchedGroups={overview ? matchedGroups : undefined}
        namedRoutes={routes.data as NamedRoute[] | undefined}
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
          artcc={artccFilter}
          groups={overviewGroups}
          onFocusFlight={focusFlight}
          onClose={() => setArtccFilter("")}
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
