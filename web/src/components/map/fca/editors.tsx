import {Button, Input} from "@ois/ui";

import type {Fca} from "@/lib/fca";
import type {MapRoute} from "@/lib/route";
import {lineNm, type LatLng} from "@/components/map/lib/geo";
import {FCA_COLORS, ROUTE_COLORS} from "@/components/map/lib/colors";

// --- FCA draft model ---

export type Phase = "draw" | "edit";

export type Draft = {
  id: string | null;
  name: string;
  color: string;
  artcc: string;
  points: LatLng[];
  dests: string;
  origins: string;
  fixes: string;
  scope: string;
  minFl: string;
  maxFl: string;
  mode: "rate" | "mit";
  rate: number;
  mit: number;
};

const list = (a: string[]) => a.join(" ");
export const parseList = (s: string) =>
  s
    .split(/[\s,]+/)
    .filter(Boolean)
    .map((t) => t.toUpperCase());
export const parseFl = (s: string): number | null => {
  const n = parseInt(s.trim(), 10);
  return Number.isFinite(n) ? n : null;
};

export function blankDraft(count: number): Draft {
  return {
    id: null,
    name: `FCA ${count + 1}`,
    color: FCA_COLORS[count % FCA_COLORS.length],
    artcc: "",
    points: [],
    dests: "",
    origins: "",
    fixes: "",
    scope: "",
    minFl: "",
    maxFl: "",
    mode: "rate",
    rate: 30,
    mit: 15,
  };
}
export function draftFrom(fca: Fca): Draft {
  return {
    id: fca.id,
    name: fca.name,
    color: fca.color,
    artcc: fca.artcc,
    points: (fca.points as LatLng[]) ?? [],
    dests: list(fca.dests),
    origins: list(fca.origins),
    fixes: list(fca.fixes),
    scope: list(fca.scope),
    minFl: fca.min_fl != null ? String(fca.min_fl) : "",
    maxFl: fca.max_fl != null ? String(fca.max_fl) : "",
    mode: fca.mode === "mit" ? "mit" : "rate",
    rate: fca.rate,
    mit: fca.mit,
  };
}

// --- Route form model ---

export type RouteForm = {
  id: string | null;
  name: string;
  route: string;
  dep: string;
  arr: string;
  color: string;
};
export function blankRouteForm(count: number): RouteForm {
  return { id: null, name: `Route ${count + 1}`, route: "", dep: "", arr: "", color: ROUTE_COLORS[count % ROUTE_COLORS.length] };
}
export function routeFormFrom(r: MapRoute): RouteForm {
  return { id: r.id, name: r.name, route: r.route, dep: r.dep, arr: r.arr, color: r.color };
}

// --- Form components ---

function Field({ label, help, children }: { label: string; help?: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{label}</span>
      {children}
      {help && <span className="text-[11px] leading-snug text-muted-foreground/80">{help}</span>}
    </label>
  );
}

export function RouteEditor({
  form,
  onChange,
  onSave,
  onCancel,
  saving,
}: {
  form: RouteForm;
  onChange: (f: RouteForm) => void;
  onSave: () => void;
  onCancel: () => void;
  saving: boolean;
}) {
  const set = <K extends keyof RouteForm>(k: K, v: RouteForm[K]) => onChange({ ...form, [k]: v });
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="text-sm font-semibold text-primary">{form.id ? "EDIT ROUTE" : "NEW ROUTE"}</div>
      <Field label="Name">
        <Input value={form.name} onChange={(e) => set("name", e.target.value)} />
      </Field>
      <Field label="Route" help="A filed-route string — fixes, navaids, airways, SID/STAR. The nav engine draws it.">
        <textarea
          value={form.route}
          onChange={(e) => set("route", e.target.value)}
          rows={3}
          placeholder="RBV Q430 BYRDD J48 MOL FLASK OZZZI2"
          className="w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm uppercase outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
      </Field>
      <div className="grid grid-cols-2 gap-3">
        <Field label="Dep" help="Optional — improves SID / preferred-route resolution.">
          <Input className="font-mono uppercase" maxLength={4} placeholder="KJFK" value={form.dep} onChange={(e) => set("dep", e.target.value)} />
        </Field>
        <Field label="Arr" help="Optional — improves STAR resolution.">
          <Input className="font-mono uppercase" maxLength={4} placeholder="KBOS" value={form.arr} onChange={(e) => set("arr", e.target.value)} />
        </Field>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Color</span>
        <div className="flex flex-wrap gap-1.5">
          {ROUTE_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              onClick={() => set("color", c)}
              className={"size-6 rounded-full " + (form.color === c ? "ring-2 ring-ring ring-offset-2 ring-offset-background" : "")}
              style={{ background: c }}
            />
          ))}
        </div>
      </div>
      <div className="flex gap-2 pt-1">
        <Button className="flex-1" onClick={onSave} disabled={!form.name.trim() || !form.route.trim() || saving}>
          Save route
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

export function DraftEditor({
  draft,
  onChange,
  onSave,
  onRedraw,
  onCancel,
  saving,
}: {
  draft: Draft;
  onChange: (d: Draft) => void;
  onSave: () => void;
  onRedraw: () => void;
  onCancel: () => void;
  saving: boolean;
}) {
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => onChange({ ...draft, [k]: v });
  return (
    <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-3">
      <div className="text-sm font-semibold text-primary">
        {draft.id ? "EDIT FCA" : "NEW FCA"} · <span className="tabular-nums">{draft.points.length}</span> pts ·{" "}
        <span className="tabular-nums">{Math.round(lineNm(draft.points))}</span> nm
      </div>

      <Field label="Name">
        <Input value={draft.name} onChange={(e) => set("name", e.target.value)} />
      </Field>
      <Field label="Destination airports" help="Space/comma-separated ICAO. Blank meters every arrival crossing the line.">
        <Input className="font-mono uppercase" placeholder="KATL KCLT · blank = all" value={draft.dests} onChange={(e) => set("dests", e.target.value)} />
      </Field>
      <Field label="Departure airports" help="Only meter flights departing these fields. Blank = any departure.">
        <Input className="font-mono uppercase" placeholder="KMCO · blank = all" value={draft.origins} onChange={(e) => set("origins", e.target.value)} />
      </Field>
      <Field label="Route fixes" help="Only meter aircraft with these fixes in their FILED route. Blank = any route.">
        <Input className="font-mono uppercase" placeholder="LAIRI · blank = all" value={draft.fixes} onChange={(e) => set("fixes", e.target.value)} />
      </Field>

      <div className="grid grid-cols-2 gap-3">
        <Field label="ARTCC tag" help="Owning facility — used by the list filter.">
          <Input className="font-mono uppercase" maxLength={4} placeholder="ZDC" value={draft.artcc} onChange={(e) => set("artcc", e.target.value)} />
        </Field>
        <Field label="Scope (ARTCCs)" help="FCA applies only to aircraft inside these centers.">
          <Input className="font-mono uppercase" placeholder="ZDC ZTL · all" value={draft.scope} onChange={(e) => set("scope", e.target.value)} />
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <Field label="Min FL">
          <Input type="number" placeholder="—" value={draft.minFl} onChange={(e) => set("minFl", e.target.value)} />
        </Field>
        <Field label="Max FL">
          <Input type="number" placeholder="—" value={draft.maxFl} onChange={(e) => set("maxFl", e.target.value)} />
        </Field>
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Constraint</span>
        <div className="flex gap-1">
          <Button size="sm" className="flex-1" variant={draft.mode === "rate" ? "default" : "secondary"} onClick={() => set("mode", "rate")}>
            Rate · ac/hr
          </Button>
          <Button size="sm" className="flex-1" variant={draft.mode === "mit" ? "default" : "secondary"} onClick={() => set("mode", "mit")}>
            MIT · nm
          </Button>
        </div>
        {draft.mode === "rate" ? (
          <>
            <Input type="number" min={0} max={240} value={draft.rate} onChange={(e) => set("rate", Number(e.target.value) || 0)} />
            <span className="text-[11px] text-muted-foreground/80">aircraft per hour → fixed time spacing (MINIT) between crossings.</span>
          </>
        ) : (
          <>
            <Input type="number" min={0} max={200} value={draft.mit} onChange={(e) => set("mit", Number(e.target.value) || 0)} />
            <span className="text-[11px] text-muted-foreground/80">miles-in-trail → spacing scaled by each aircraft&apos;s crossing speed.</span>
          </>
        )}
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Color</span>
        <div className="flex flex-wrap gap-1.5">
          {FCA_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              onClick={() => set("color", c)}
              className={"size-6 rounded-full " + (draft.color === c ? "ring-2 ring-ring ring-offset-2 ring-offset-background" : "")}
              style={{ background: c }}
            />
          ))}
        </div>
      </div>

      <div className="mt-auto flex flex-col gap-2 pt-2">
        <div className="flex gap-2">
          <Button variant="secondary" className="flex-1" onClick={onRedraw}>
            ↻ Redraw line
          </Button>
          <Button className="flex-1" onClick={onSave} disabled={draft.points.length < 2 || saving}>
            Save FCA
          </Button>
        </div>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

export function Kbd({ children }: { children: React.ReactNode }) {
  return <kbd className="rounded border bg-muted px-1.5 py-0.5 font-mono text-xs">{children}</kbd>;
}
