import {Button, Modal, usePrompt} from "@ois/ui";
import {Plus, X} from "lucide-react";

import {AGGREGATES, CHART_TYPES, colorAt, labelOf, type Series, TOP_OPTIONS} from "./chart-shared";
import {AIRPORT_KEY, type DataSource} from "./sources";
import type {ChartWidget as ChartWidgetT} from "./types";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="mb-5">
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </h3>
      <div className="flex flex-col gap-3">{children}</div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}

function Select({
  value,
  onChange,
  children,
}: {
  value: string;
  onChange: (v: string) => void;
  children: React.ReactNode;
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className="h-8 rounded-md border border-input bg-background px-2 text-sm capitalize outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      {children}
    </select>
  );
}

export function ChartConfigPanel({
  source,
  widget,
  series,
  categories,
  multiAirport,
  onChange,
  onClose,
}: {
  source: DataSource;
  widget: ChartWidgetT;
  series: Series[];
  /** Distinct x-axis categories currently rendered, in order — drives the category-color rows. */
  categories: string[];
  multiAirport: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
  onClose: () => void;
}) {
  const prompt = usePrompt();
  const set = (patch: Record<string, unknown>) => onChange(widget.id, patch);

  const aggregate = widget.aggregate ?? "none";
  const nums = source.fields.filter((f) => f.type === "number");
  const icaos = widget.params?.icaos?.length
    ? widget.params.icaos
    : widget.params?.icao
      ? [widget.params.icao]
      : [];
  const splitMode = multiAirport && widget.x !== AIRPORT_KEY;
  const xOptions = [
    ...source.fields.map((f) => ({ key: f.key, label: f.label })),
    ...(multiAirport ? [{ key: AIRPORT_KEY, label: "Airport" }] : []),
  ];
  const ySet = new Set(widget.y);
  const thresholds = widget.thresholds ?? [];
  const categoryColors = widget.categoryColors ?? {};
  const showCategoryColors =
    (widget.chartType === "bar" || widget.chartType === "scatter") &&
    aggregate !== "none" &&
    categories.length > 0;
  const setCategoryColor = (cat: string, hex: string) =>
    set({ categoryColors: { ...categoryColors, [cat]: hex } });
  const clearCategoryColor = (cat: string) => {
    const next = { ...categoryColors };
    delete next[cat];
    set({ categoryColors: next });
  };

  const addAirport = async () => {
    const raw = await prompt({ title: "Add airport", label: "ICAO", placeholder: "KBOS" });
    if (!raw) return;
    const ic = raw.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (ic.length >= 3 && !icaos.includes(ic)) {
      set({ params: { ...widget.params, icaos: [...icaos, ic] } });
    }
  };
  const removeAirport = (ic: string) => {
    if (icaos.length > 1) set({ params: { ...widget.params, icaos: icaos.filter((x) => x !== ic) } });
  };
  const toggleY = (key: string) => {
    const next = nums.map((f) => f.key).filter((k) => (k === key ? !ySet.has(k) : ySet.has(k)));
    if (next.length > 0) set({ y: next });
  };
  const setThreshold = (i: number, patch: { value?: number; color?: string }) =>
    set({ thresholds: thresholds.map((t, idx) => (idx === i ? { ...t, ...patch } : t)) });
  const addThreshold = () => set({ thresholds: [...thresholds, { value: 0, color: "#f87171" }] });
  const removeThreshold = (i: number) =>
    set({ thresholds: thresholds.filter((_, idx) => idx !== i) });

  return (
    <Modal open onClose={onClose} title="Chart settings" placement="right">
          <Section title="Data">
            {source.needsIcao && (
              <Field label="Airports">
                <div className="flex flex-wrap items-center gap-1.5">
                  {icaos.map((ic) => (
                    <span
                      key={ic}
                      className="flex items-center gap-1 rounded border bg-muted/40 px-2 py-0.5 font-mono text-xs"
                    >
                      {ic}
                      <button
                        type="button"
                        onClick={() => removeAirport(ic)}
                        className="text-muted-foreground hover:text-destructive"
                        aria-label={`Remove ${ic}`}
                      >
                        <X className="size-3" />
                      </button>
                    </span>
                  ))}
                  <Button size="sm" variant="secondary" className="h-6" onClick={() => void addAirport()}>
                    <Plus className="size-3" />
                    Add
                  </Button>
                </div>
              </Field>
            )}

            <Field label="Group by (X axis)">
              <Select value={widget.x} onChange={(v) => set({ x: v })}>
                {xOptions.map((o) => (
                  <option key={o.key} value={o.key}>
                    {o.label}
                  </option>
                ))}
              </Select>
            </Field>

            <Field label="Aggregate">
              <Select value={aggregate} onChange={(v) => set({ aggregate: v })}>
                {AGGREGATES.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.label}
                  </option>
                ))}
              </Select>
            </Field>

            {aggregate !== "count" &&
              aggregate !== "none" &&
              (splitMode ? (
                <Field label="Metric">
                  <Select value={widget.y[0] ?? ""} onChange={(v) => set({ y: [v] })}>
                    {nums.map((f) => (
                      <option key={f.key} value={f.key}>
                        {f.label}
                      </option>
                    ))}
                  </Select>
                </Field>
              ) : (
                <Field label="Y series">
                  <div className="flex flex-col gap-1">
                    {nums.map((f) => (
                      <label key={f.key} className="flex items-center gap-2 text-sm">
                        <input
                          type="checkbox"
                          checked={ySet.has(f.key)}
                          onChange={() => toggleY(f.key)}
                        />
                        {f.label}
                      </label>
                    ))}
                  </div>
                </Field>
              ))}

            {aggregate !== "none" && (
              <Field label="Limit">
                <Select value={String(widget.topN ?? 0)} onChange={(v) => set({ topN: Number(v) })}>
                  {TOP_OPTIONS.map((n) => (
                    <option key={n} value={n}>
                      {n === 0 ? "All" : `Top ${n}`}
                    </option>
                  ))}
                </Select>
              </Field>
            )}
          </Section>

          <Section title="Display">
            <Field label="Chart type">
              <Select value={widget.chartType} onChange={(v) => set({ chartType: v })}>
                {CHART_TYPES.map((t) => (
                  <option key={t} value={t}>
                    {t}
                  </option>
                ))}
              </Select>
            </Field>

            <Field label="Y scaling">
              <Select
                value={widget.normalize ? "norm" : "actual"}
                onChange={(v) => set({ normalize: v === "norm" })}
              >
                <option value="actual">Actual values</option>
                <option value="norm">Normalize %</option>
              </Select>
            </Field>

            {series.length > 0 && (
              <Field label="Series colours">
                <div className="flex flex-col gap-1.5">
                  {series.map((s, i) => (
                    <div key={s.key} className="flex items-center gap-2 text-sm">
                      <input
                        type="color"
                        value={widget.colors?.[s.key] ?? colorAt(i)}
                        onChange={(e) => set({ colors: { ...widget.colors, [s.key]: e.target.value } })}
                        className="h-6 w-8 cursor-pointer rounded border bg-transparent"
                        aria-label={`Colour for ${s.label}`}
                      />
                      <span className="truncate">{s.label}</span>
                    </div>
                  ))}
                </div>
              </Field>
            )}

            {showCategoryColors && (
              <Field label="Category colours">
                <div className="flex flex-col gap-1.5">
                  {categories.map((cat, i) => (
                    <div key={cat} className="flex items-center gap-2 text-sm">
                      <input
                        type="color"
                        value={categoryColors[cat] ?? colorAt(i)}
                        onChange={(e) => setCategoryColor(cat, e.target.value)}
                        className="h-6 w-8 cursor-pointer rounded border bg-transparent"
                        aria-label={`Colour for ${cat}`}
                      />
                      <span className="truncate">{cat}</span>
                      {categoryColors[cat] != null && (
                        <button
                          type="button"
                          onClick={() => clearCategoryColor(cat)}
                          className="ml-auto text-muted-foreground hover:text-destructive"
                          aria-label={`Reset ${cat} to its series colour`}
                        >
                          <X className="size-3.5" />
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </Field>
            )}
          </Section>

          <Section title="Thresholds">
            {widget.normalize ? (
              <p className="text-xs text-muted-foreground">Not available while normalized.</p>
            ) : (
              <div className="flex flex-col gap-1.5">
                {thresholds.map((t, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <input
                      type="number"
                      value={t.value}
                      onChange={(e) => setThreshold(i, { value: Number(e.target.value) })}
                      className="h-8 w-24 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    />
                    <input
                      type="color"
                      value={t.color}
                      onChange={(e) => setThreshold(i, { color: e.target.value })}
                      className="h-8 w-8 cursor-pointer rounded border bg-transparent"
                      aria-label="Threshold colour"
                    />
                    <button
                      type="button"
                      onClick={() => removeThreshold(i)}
                      className="ml-auto text-muted-foreground hover:text-destructive"
                      aria-label="Remove threshold"
                    >
                      <X className="size-4" />
                    </button>
                  </div>
                ))}
                <Button size="sm" variant="secondary" className="mt-1 self-start" onClick={addThreshold}>
                  <Plus className="size-3.5" />
                  Add threshold
                </Button>
              </div>
            )}
          </Section>
    </Modal>
  );
}
