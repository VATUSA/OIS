import {useEffect, useMemo, useRef, useState} from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import {Button, Input} from "@ois/ui";
import {Check, Pencil, Plus, Trash2, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {
  type Fca,
  toUpsert,
  useCreateFca,
  useDeleteFca,
  useFcas,
  useFcaTraffic,
  useTraffic,
  useUpdateFca,
} from "@/lib/fca";
import {FcaDetail} from "@/pages/fca/detail";

const FCA_COLORS = [
  "#f59e0b",
  "#ec4899",
  "#38bdf8",
  "#22c55e",
  "#a855f7",
  "#f97316",
  "#14b8a6",
  "#ef4444",
];

type LatLng = [number, number];

type Draft = {
  name: string;
  color: string;
  artcc: string;
  points: LatLng[];
  mode: "rate" | "mit";
  rate: number;
  mit: number;
};

function draftFrom(fca: Fca): Draft {
  return {
    name: fca.name,
    color: fca.color,
    artcc: fca.artcc,
    points: (fca.points as LatLng[]) ?? [],
    mode: fca.mode === "mit" ? "mit" : "rate",
    rate: fca.rate,
    mit: fca.mit,
  };
}

function aircraftIcon(heading: number) {
  return L.divIcon({
    className: "",
    html: `<svg width="12" height="12" viewBox="0 0 12 12" style="transform: rotate(${heading}deg)"><path d="M6 0 L10.5 11 L6 8.5 L1.5 11 Z" fill="#22d3ee" fill-opacity="0.85"/></svg>`,
    iconSize: [12, 12],
    iconAnchor: [6, 6],
  });
}

function vertexIcon(color: string) {
  return L.divIcon({
    className: "",
    html: `<div style="width:12px;height:12px;border-radius:50%;background:#fff;border:2px solid ${color};box-shadow:0 0 0 1px rgba(0,0,0,.4)"></div>`,
    iconSize: [12, 12],
    iconAnchor: [6, 6],
  });
}

function labelIcon(color: string, name: string) {
  return L.divIcon({
    className: "",
    html: `<span style="white-space:nowrap;font:600 12px ui-monospace,monospace;color:${color};text-shadow:0 1px 2px #000">▮ ${name}</span>`,
    iconSize: [0, 0],
    iconAnchor: [-6, 6],
  });
}

export function FcaPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "flow.fca.read");
  const canEdit = hasPermission(me, "flow.fca.update");
  const canDelete = hasPermission(me, "flow.fca.delete");

  const fcas = useFcas();
  const traffic = useTraffic();
  const createFca = useCreateFca();
  const updateFca = useUpdateFca();
  const deleteFca = useDeleteFca();

  const [draft, setDraft] = useState<Draft | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

  const fcaTraffic = useFcaTraffic(draft ? null : selectedId);

  // --- Leaflet refs ---
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<L.Map | null>(null);
  const aircraftLayer = useRef<L.LayerGroup | null>(null);
  const fcaLayer = useRef<L.LayerGroup | null>(null);
  const matchedLayer = useRef<L.LayerGroup | null>(null);
  const draftLayer = useRef<L.LayerGroup | null>(null);
  const draftRef = useRef<Draft | null>(null);
  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  // Init map once.
  useEffect(() => {
    if (!containerRef.current || mapRef.current) return;
    const map = L.map(containerRef.current, {
      zoomControl: true,
      worldCopyJump: false,
      attributionControl: true,
    }).setView([38.5, -77], 6);
    L.tileLayer(
      "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png",
      {
        maxZoom: 14,
        attribution: "© OpenStreetMap, © CARTO · traffic: VATSIM",
      },
    ).addTo(map);
    aircraftLayer.current = L.layerGroup().addTo(map);
    fcaLayer.current = L.layerGroup().addTo(map);
    matchedLayer.current = L.layerGroup().addTo(map);
    draftLayer.current = L.layerGroup().addTo(map);
    map.on("click", (e: L.LeafletMouseEvent) => {
      const d = draftRef.current;
      if (!d) return;
      setDraft({ ...d, points: [...d.points, [e.latlng.lat, e.latlng.lng]] });
    });
    mapRef.current = map;
    setTimeout(() => map.invalidateSize(), 100);
    return () => {
      map.remove();
      mapRef.current = null;
    };
  }, []);

  // Redraw live aircraft.
  useEffect(() => {
    const layer = aircraftLayer.current;
    if (!layer) return;
    layer.clearLayers();
    for (const ac of traffic.data ?? []) {
      L.marker([ac.lat, ac.lon], {
        icon: aircraftIcon(ac.heading),
        interactive: false,
        keyboard: false,
      }).addTo(layer);
    }
  }, [traffic.data]);

  // Redraw saved FCAs (skip the one being edited — it's on the draft layer).
  useEffect(() => {
    const layer = fcaLayer.current;
    if (!layer) return;
    layer.clearLayers();
    for (const fca of fcas.data ?? []) {
      if (fca.id === editingId) continue;
      const pts = fca.points as LatLng[];
      if (!pts || pts.length < 2) continue;
      const selected = fca.id === selectedId;
      L.polyline(pts, {
        color: fca.color,
        weight: selected ? 6 : fca.enabled ? 4 : 2,
        opacity: fca.enabled ? 0.9 : 0.35,
        dashArray: fca.enabled ? undefined : "4 6",
      })
        .on("click", (e) => {
          L.DomEvent.stop(e);
          setSelectedId((cur) => (cur === fca.id ? null : fca.id));
        })
        .addTo(layer);
      const mid = pts[Math.floor(pts.length / 2)];
      L.marker(mid, {
        icon: labelIcon(fca.color, fca.name),
        interactive: false,
        keyboard: false,
      }).addTo(layer);
    }
  }, [fcas.data, editingId, selectedId]);

  // Redraw matched (crossing) traffic for the selected FCA.
  useEffect(() => {
    const layer = matchedLayer.current;
    if (!layer) return;
    layer.clearLayers();
    if (draft) return;
    for (const f of fcaTraffic.data ?? []) {
      L.circleMarker([f.cross_lat, f.cross_lon], {
        radius: 3,
        color: "#ffffff",
        weight: 1,
        fillColor: "#ffffff",
        fillOpacity: 0.9,
      }).addTo(layer);
      if (f.lat !== 0 || f.lon !== 0) {
        L.circleMarker([f.lat, f.lon], {
          radius: 5,
          color: "#22c55e",
          weight: 2,
          fillColor: "#22c55e",
          fillOpacity: 0.45,
        })
          .bindTooltip(`${f.callsign} · ${f.dep}→${f.arr}`, { direction: "top" })
          .addTo(layer);
      }
    }
  }, [fcaTraffic.data, draft]);

  // Redraw the working draft (polyline + draggable vertex handles).
  useEffect(() => {
    const layer = draftLayer.current;
    if (!layer) return;
    layer.clearLayers();
    if (!draft) return;
    if (draft.points.length >= 2) {
      L.polyline(draft.points, {
        color: draft.color,
        weight: 4,
        dashArray: "6 6",
      }).addTo(layer);
    }
    draft.points.forEach((pt, i) => {
      const handle = L.marker(pt, {
        draggable: true,
        icon: vertexIcon(draft.color),
      });
      handle.on("dragend", (e) => {
        const ll = (e.target as L.Marker).getLatLng();
        setDraft((prev) =>
          prev
            ? {
                ...prev,
                points: prev.points.map((p, idx) =>
                  idx === i ? [ll.lat, ll.lng] : p,
                ),
              }
            : prev,
        );
      });
      handle.addTo(layer);
    });
  }, [draft]);

  const startNew = () => {
    const color = FCA_COLORS[(fcas.data?.length ?? 0) % FCA_COLORS.length];
    setSelectedId(null);
    setEditingId(null);
    setDraft({
      name: `FCA ${(fcas.data?.length ?? 0) + 1}`,
      color,
      artcc: "",
      points: [],
      mode: "rate",
      rate: 30,
      mit: 15,
    });
  };

  const startEdit = (fca: Fca) => {
    setSelectedId(null);
    setEditingId(fca.id);
    setDraft(draftFrom(fca));
  };

  const cancel = () => {
    setDraft(null);
    setEditingId(null);
  };

  const save = () => {
    if (!draft || draft.points.length < 2) return;
    const body = {
      name: draft.name.trim() || "FCA",
      color: draft.color,
      artcc: draft.artcc,
      points: draft.points,
      mode: draft.mode,
      rate: draft.rate,
      mit: draft.mit,
    };
    if (editingId) {
      updateFca.mutate({ id: editingId, body }, { onSuccess: cancel });
    } else {
      createFca.mutate(body, { onSuccess: cancel });
    }
  };

  const toggleEnabled = (fca: Fca) => {
    updateFca.mutate({ id: fca.id, body: { ...toUpsert(fca), enabled: !fca.enabled } });
  };

  const shown = useMemo(() => {
    const q = filter.trim().toUpperCase();
    return (fcas.data ?? []).filter(
      (f) =>
        !q ||
        f.name.toUpperCase().includes(q) ||
        f.artcc.toUpperCase().includes(q),
    );
  }, [fcas.data, filter]);

  const selectedFca = fcas.data?.find((f) => f.id === selectedId);

  if (!canRead) {
    return (
      <div className="flex h-[calc(100vh-3.5rem)] items-center justify-center text-sm text-muted-foreground">
        You don&apos;t have flow access.
      </div>
    );
  }

  return (
    <div className="flex h-[calc(100vh-3.5rem)]">
      {/* Sidebar */}
      <aside className="flex w-80 shrink-0 flex-col border-r bg-background">
        <div className="flex items-center justify-between border-b px-4 py-3">
          <span className="text-sm font-semibold uppercase tracking-wide">
            Flow Constrained Areas
          </span>
        </div>

        {canEdit && !draft && (
          <div className="border-b p-3">
            <Button className="w-full" onClick={startNew}>
              <Plus />
              New FCA
            </Button>
          </div>
        )}

        {draft ? (
          <DraftEditor
            draft={draft}
            editing={!!editingId}
            onChange={setDraft}
            onSave={save}
            onCancel={cancel}
            saving={createFca.isPending || updateFca.isPending}
          />
        ) : (
          <>
            <div className="border-b p-3">
              <Input
                placeholder="filter — name or ARTCC…"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
              />
            </div>
            <div className="flex-1 overflow-y-auto">
              {!fcas.data ? (
                <p className="p-4 text-sm text-muted-foreground">Loading…</p>
              ) : shown.length === 0 ? (
                <p className="p-4 text-sm text-muted-foreground">
                  No FCAs yet. {canEdit && "Draw one with “New FCA”."}
                </p>
              ) : (
                <ul>
                  {shown.map((fca) => (
                    <li
                      key={fca.id}
                      className={
                        "flex items-center gap-2 border-b px-3 py-2 text-sm " +
                        (fca.id === selectedId ? "bg-accent/40" : "")
                      }
                    >
                      <button
                        type="button"
                        title={fca.enabled ? "Enabled" : "Disabled"}
                        onClick={() => canEdit && toggleEnabled(fca)}
                        className="size-3 shrink-0 rounded-full"
                        style={{
                          background: fca.enabled ? fca.color : "transparent",
                          border: `2px solid ${fca.color}`,
                        }}
                      />
                      <button
                        type="button"
                        onClick={() =>
                          setSelectedId((cur) => (cur === fca.id ? null : fca.id))
                        }
                        className="flex-1 truncate text-left font-mono"
                      >
                        {fca.name}
                        {fca.artcc && (
                          <span className="ml-1.5 text-xs text-muted-foreground">
                            {fca.artcc}
                          </span>
                        )}
                      </button>
                      {fca.id === selectedId && (
                        <span className="shrink-0 rounded bg-primary/15 px-1.5 text-xs font-medium tabular-nums text-primary">
                          {fcaTraffic.data?.length ?? "…"}
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
                        <button
                          type="button"
                          title="Delete"
                          onClick={() => deleteFca.mutate(fca.id)}
                          className="text-muted-foreground hover:text-destructive"
                        >
                          <Trash2 className="size-3.5" />
                        </button>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </>
        )}

        <div className="border-t px-4 py-2 text-xs text-muted-foreground">
          {traffic.data ? `traffic ${traffic.data.length}` : "traffic…"}
        </div>
      </aside>

      {/* Map */}
      {/* `isolate` traps Leaflet's internal z-indexes so the navbar dropdowns
          (portaled, higher z) sit in front of the map. */}
      <div ref={containerRef} className="relative flex-1 isolate" />

      {/* Detail board for the selected FCA (metering ladder + strips). */}
      {selectedFca && !draft && (
        <FcaDetail
          fca={selectedFca}
          flights={fcaTraffic.data}
          canEdit={canEdit}
        />
      )}
    </div>
  );
}

function DraftEditor({
  draft,
  editing,
  onChange,
  onSave,
  onCancel,
  saving,
}: {
  draft: Draft;
  editing: boolean;
  onChange: (d: Draft) => void;
  onSave: () => void;
  onCancel: () => void;
  saving: boolean;
}) {
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) =>
    onChange({ ...draft, [k]: v });

  return (
    <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-3">
      <p className="text-xs text-muted-foreground">
        {editing ? "Editing" : "Drawing"} — click the map to add points, drag
        points to move. Need at least 2.
      </p>

      <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        Name
        <Input value={draft.name} onChange={(e) => set("name", e.target.value)} />
      </label>

      <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        ARTCC
        <Input
          className="font-mono uppercase"
          maxLength={4}
          value={draft.artcc}
          onChange={(e) => set("artcc", e.target.value.toUpperCase())}
        />
      </label>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Color
        </span>
        <div className="flex flex-wrap gap-1.5">
          {FCA_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              onClick={() => set("color", c)}
              className={
                "size-6 rounded-full " +
                (draft.color === c ? "ring-2 ring-ring ring-offset-2 ring-offset-background" : "")
              }
              style={{ background: c }}
            />
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Metering
        </span>
        <div className="flex gap-1">
          <Button
            size="sm"
            variant={draft.mode === "rate" ? "default" : "secondary"}
            onClick={() => set("mode", "rate")}
          >
            Rate
          </Button>
          <Button
            size="sm"
            variant={draft.mode === "mit" ? "default" : "secondary"}
            onClick={() => set("mode", "mit")}
          >
            MIT
          </Button>
        </div>
      </div>

      {draft.mode === "rate" ? (
        <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Rate (ac/hr)
          <Input
            type="number"
            min={0}
            max={240}
            value={draft.rate}
            onChange={(e) => set("rate", Number(e.target.value) || 0)}
          />
        </label>
      ) : (
        <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          MIT (nm)
          <Input
            type="number"
            min={0}
            max={200}
            value={draft.mit}
            onChange={(e) => set("mit", Number(e.target.value) || 0)}
          />
        </label>
      )}

      <div className="mt-auto flex gap-2 pt-2">
        <Button
          className="flex-1"
          onClick={onSave}
          disabled={draft.points.length < 2 || saving}
        >
          <Check />
          Save
        </Button>
        <Button variant="secondary" onClick={onCancel}>
          <X />
          Cancel
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {draft.points.length} point{draft.points.length === 1 ? "" : "s"}
      </p>
    </div>
  );
}
