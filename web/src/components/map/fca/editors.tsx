import {useEffect, useState} from "react";
import {Button, cn, Input, SegmentedControl, Textarea} from "@ois/ui";

import {useValidateFixes} from "@/lib/fca";
import type {Fca} from "@/lib/fca";
import type {MapRoute} from "@/lib/route";
import {lineNm, type LatLng} from "@/components/map/lib/geo";
import {readFcaColors, readRouteColors, useFcaColors, useRouteColors} from "@/components/map/lib/colors";

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

/** A new FCA draft; `colors` is the swatch cycle (`useFcaColors()`), read now when omitted. */
export function blankDraft(count: number, colors: string[] = readFcaColors()): Draft {
  return {
    id: null,
    name: `FCA ${count + 1}`,
    color: colors[count % colors.length],
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
/** A new route form; `colors` is the swatch cycle (`useRouteColors()`), read now when omitted. */
export function blankRouteForm(count: number, colors: string[] = readRouteColors()): RouteForm {
  return { id: null, name: `Route ${count + 1}`, route: "", dep: "", arr: "", color: colors[count % colors.length] };
}
export function routeFormFrom(r: MapRoute): RouteForm {
  return { id: r.id, name: r.name, route: r.route, dep: r.dep, arr: r.arr, color: r.color };
}

// --- Form components ---

function Field({ label, help, children }: { label: string; help?: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-ink-3">{label}</span>
      {children}
      {help && <span className="text-[11px] leading-snug text-ink-3">{help}</span>}
    </label>
  );
}

/** Colour swatches from the token palette; a saved colour outside it still shows (titled with its hex). */
function Swatches({ colors, value, onChange }: { colors: string[]; value: string; onChange: (c: string) => void }) {
  const known = colors.some((c) => c.toLowerCase() === value.toLowerCase());
  const all = known || !value ? colors : [...colors, value];
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-ink-3">Color</span>
      <div className="flex flex-wrap gap-1.5">
        {all.map((c) => {
          const on = c.toLowerCase() === value.toLowerCase();
          return (
            <button
              key={c}
              type="button"
              title={c}
              aria-label={`Color ${c}`}
              aria-pressed={on}
              onClick={() => onChange(c)}
              className={cn("size-6 rounded-full", on && "ring-2 ring-ring ring-offset-2 ring-offset-panel")}
              style={{ background: c }}
            />
          );
        })}
      </div>
    </div>
  );
}

/** Route-fixes input that flags entries which aren't real nav fixes (typos silently exclude traffic). */
function FixesField({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), 400);
    return () => clearTimeout(t);
  }, [value]);
  const unknown = useValidateFixes(debounced).data?.unknown ?? [];

  return (
    <Field label="Route fixes" help="Only meter aircraft with these fixes in their FILED route. Blank = any route.">
      <Input className="font-mono uppercase" placeholder="LAIRI · blank = all" value={value} onChange={(e) => onChange(e.target.value)} />
      {unknown.length > 0 && (
        <span className="text-[11px] leading-snug text-danger">
          Not a known fix: {unknown.join(", ")} — check for a typo; it won&apos;t match any traffic.
        </span>
      )}
    </Field>
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
  const colors = useRouteColors();
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="text-sm font-semibold text-brand-ink">{form.id ? "EDIT ROUTE" : "NEW ROUTE"}</div>
      <Field label="Name">
        <Input value={form.name} onChange={(e) => set("name", e.target.value)} />
      </Field>
      <Field label="Route" help="A filed-route string — fixes, navaids, airways, SID/STAR. The nav engine draws it.">
        <Textarea
          value={form.route}
          onChange={(e) => set("route", e.target.value)}
          rows={3}
          placeholder="RBV Q430 BYRDD J48 MOL FLASK OZZZI2"
          className="min-h-0 font-mono uppercase"
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
      <Swatches colors={colors} value={form.color} onChange={(c) => set("color", c)} />
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

const MODE_OPTIONS = [
  { value: "rate", label: "Rate · ac/hr" },
  { value: "mit", label: "MIT · nm" },
] as const;

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
  const colors = useFcaColors();
  return (
    <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-3">
      <div className="text-sm font-semibold text-brand-ink">
        {draft.id ? "EDIT FCA" : "NEW FCA"} · <span className="font-mono">{draft.points.length}</span> pts ·{" "}
        <span className="font-mono">{Math.round(lineNm(draft.points))}</span> nm
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
      <FixesField value={draft.fixes} onChange={(v) => set("fixes", v)} />


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
        <span className="text-xs font-semibold uppercase tracking-wide text-ink-3">Constraint</span>
        <SegmentedControl
          aria-label="Constraint"
          value={draft.mode}
          onChange={(m) => set("mode", m)}
          options={MODE_OPTIONS}
          className="w-full [&>button]:flex-1 [&>button]:justify-center"
        />
        {draft.mode === "rate" ? (
          <>
            <Input type="number" min={0} max={240} value={draft.rate} onChange={(e) => set("rate", Number(e.target.value) || 0)} />
            <span className="text-[11px] text-ink-3">aircraft per hour → fixed time spacing (MINIT) between crossings.</span>
          </>
        ) : (
          <>
            <Input type="number" min={0} max={200} value={draft.mit} onChange={(e) => set("mit", Number(e.target.value) || 0)} />
            <span className="text-[11px] text-ink-3">miles-in-trail → spacing scaled by each aircraft&apos;s crossing speed.</span>
          </>
        )}
      </div>

      <Swatches colors={colors} value={draft.color} onChange={(c) => set("color", c)} />

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
  return <kbd className="rounded-xs border border-line bg-chip px-1.5 py-0.5 font-mono text-xs text-ink-2">{children}</kbd>;
}
