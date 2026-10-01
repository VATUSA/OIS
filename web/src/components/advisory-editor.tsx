import {Button, Input, SegmentedControl, Select} from "@ois/ui";
import {Plus, X} from "lucide-react";

import type {Reroute, RerouteRoutes, RerouteValidBasis} from "@/lib/advisories";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-ink-2">
      {label}
      {children}
    </label>
  );
}

const BASES: { value: RerouteValidBasis; label: string }[] = [
  { value: "fca_entry_time", label: "FCA entry time" },
  { value: "etd", label: "ETD" },
];

/**
 * The route table. Plain repeatable inputs on purpose: the author fills fields and the **backend**
 * renders the aligned document (`advisory_body` → `render_reroute`), so nobody hand-pads text
 * (VATUSA/OIS#460 AC4). `><` mandatory-segment markers are typed into `route` verbatim — the renderer
 * stores and emits them untouched and never validates them.
 */
function RoutesEditor({
  value,
  onChange,
}: {
  value: RerouteRoutes;
  onChange: (r: RerouteRoutes) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <SegmentedControl
        aria-label="Route table shape"
        value={value.kind}
        onChange={(kind) =>
          onChange(
            kind === "single"
              ? { kind: "single", rows: [{ orig: "", dest: "", route: "" }] }
              : { kind: "segmented", origin: [{ orig: "", route: "" }], destination: [{ orig: "", route: "" }] },
          )
        }
        options={[
          { value: "single", label: "Single" },
          { value: "segmented", label: "Segmented" },
        ]}
      />

      {value.kind === "single" ? (
        <div className="flex flex-col gap-2">
          {value.rows.map((row, i) => (
            <div key={i} className="grid grid-cols-[6rem_6rem_1fr_auto] items-end gap-2">
              <Field label={i === 0 ? "Orig" : ""}>
                <Input
                  aria-label={`Row ${i + 1} origin`}
                  value={row.orig}
                  onChange={(e) =>
                    onChange({
                      ...value,
                      rows: value.rows.map((r, j) => (i === j ? { ...r, orig: e.target.value } : r)),
                    })
                  }
                />
              </Field>
              <Field label={i === 0 ? "Dest" : ""}>
                <Input
                  aria-label={`Row ${i + 1} destination`}
                  value={row.dest}
                  onChange={(e) =>
                    onChange({
                      ...value,
                      rows: value.rows.map((r, j) => (i === j ? { ...r, dest: e.target.value } : r)),
                    })
                  }
                />
              </Field>
              <Field label={i === 0 ? "Route" : ""}>
                <Input
                  aria-label={`Row ${i + 1} route`}
                  value={row.route}
                  onChange={(e) =>
                    onChange({
                      ...value,
                      rows: value.rows.map((r, j) => (i === j ? { ...r, route: e.target.value } : r)),
                    })
                  }
                />
              </Field>
              <Button
                variant="ghost"
                aria-label={`Remove row ${i + 1}`}
                // The last row never goes: `routes` is required, so an empty table cannot be saved.
                disabled={value.rows.length === 1}
                onClick={() => onChange({ ...value, rows: value.rows.filter((_, j) => j !== i) })}
              >
                <X className="size-4" />
              </Button>
            </div>
          ))}
          <Button
            variant="secondary"
            onClick={() => onChange({ ...value, rows: [...value.rows, { orig: "", dest: "", route: "" }] })}
          >
            <Plus className="size-4" /> Add row
          </Button>
        </div>
      ) : (
        (["origin", "destination"] as const).map((side) => (
          <div key={side} className="flex flex-col gap-2">
            <span className="text-xs font-semibold uppercase tracking-wide text-ink-3">{side}</span>
            {value[side].map((seg, i) => (
              <div key={i} className="grid grid-cols-[6rem_1fr_auto] items-end gap-2">
                <Field label={i === 0 ? "Orig" : ""}>
                  <Input
                    aria-label={`${side} ${i + 1} origin`}
                    value={seg.orig}
                    onChange={(e) =>
                      onChange({
                        ...value,
                        [side]: value[side].map((s, j) => (i === j ? { ...s, orig: e.target.value } : s)),
                      })
                    }
                  />
                </Field>
                <Field label={i === 0 ? "Route" : ""}>
                  <Input
                    aria-label={`${side} ${i + 1} route`}
                    value={seg.route}
                    onChange={(e) =>
                      onChange({
                        ...value,
                        [side]: value[side].map((s, j) => (i === j ? { ...s, route: e.target.value } : s)),
                      })
                    }
                  />
                </Field>
                <Button
                  variant="ghost"
                  aria-label={`Remove ${side} ${i + 1}`}
                  disabled={value[side].length === 1}
                  onClick={() =>
                    onChange({ ...value, [side]: value[side].filter((_, j) => j !== i) })
                  }
                >
                  <X className="size-4" />
                </Button>
              </div>
            ))}
            <Button
              variant="secondary"
              onClick={() => onChange({ ...value, [side]: [...value[side], { orig: "", route: "" }] })}
            >
              <Plus className="size-4" /> Add {side} segment
            </Button>
          </div>
        ))
      )}
    </div>
  );
}

/**
 * Structured editor for a Reroute advisory (VATUSA/OIS#460).
 *
 * There is deliberately **no preview rendered here.** The document is produced by the backend —
 * `advisory_body` derives it from `structured` on create and update, so "the fields are the source of
 * truth and the document is their rendering, and the two can never drift". The page previews the saved
 * draft's `body`, which is what will actually post. Rendering it in TypeScript instead would
 * re-introduce exactly the two-language divergence #455 was filed to remove.
 */
export function AdvisoryEditor({ value, onChange }: { value: Reroute; onChange: (r: Reroute) => void }) {
  const set = (patch: Partial<Reroute>) => onChange({ ...value, ...patch });

  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Field label="Name">
          <Input
            placeholder="CAMRN ARRIVALS"
            value={value.name}
            onChange={(e) => set({ name: e.target.value })}
          />
        </Field>
        <Field label="Header qualifier">
          <Input
            placeholder="FCA RQD/FL"
            value={value.header}
            onChange={(e) => set({ header: e.target.value })}
          />
        </Field>
        <Field label="Impacted area">
          <Input
            placeholder="ZNY"
            value={value.impacted_area}
            onChange={(e) => set({ impacted_area: e.target.value })}
          />
        </Field>
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Field label="Valid basis">
          <Select
            wrapperClassName="w-full"
            value={value.valid.basis}
            onChange={(e) =>
              set({ valid: { ...value.valid, basis: e.target.value as RerouteValidBasis } })
            }
          >
            {BASES.map((b) => (
              <option key={b.value} value={b.value}>
                {b.label}
              </option>
            ))}
          </Select>
        </Field>
        {/* DDHHMM, kept as written. The model's own comment says a round-trip through a timestamp
            would have to invent a month and year the document never states. */}
        <Field label="Valid from (DDHHMM)">
          <Input
            placeholder="141800"
            value={value.valid.from}
            onChange={(e) => set({ valid: { ...value.valid, from: e.target.value } })}
          />
        </Field>
        <Field label="Valid to (DDHHMM)">
          <Input
            placeholder="142359"
            value={value.valid.to}
            onChange={(e) => set({ valid: { ...value.valid, to: e.target.value } })}
          />
        </Field>
      </div>

      <RoutesEditor value={value.routes} onChange={(routes) => set({ routes })} />

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Field label="Reason">
          <Input value={value.reason ?? ""} onChange={(e) => set({ reason: e.target.value })} />
        </Field>
        <Field label="Include traffic">
          <Input
            value={value.include_traffic ?? ""}
            onChange={(e) => set({ include_traffic: e.target.value })}
          />
        </Field>
        <Field label="Facilities included">
          <Input
            value={value.facilities_included ?? ""}
            onChange={(e) => set({ facilities_included: e.target.value })}
          />
        </Field>
        <Field label="Associated restrictions">
          <Input
            value={value.associated_restrictions ?? ""}
            onChange={(e) => set({ associated_restrictions: e.target.value })}
          />
        </Field>
        <Field label="Modifications">
          <Input
            value={value.modifications ?? ""}
            onChange={(e) => set({ modifications: e.target.value })}
          />
        </Field>
        <Field label="Probability of extension">
          <Input
            value={value.probability_of_extension ?? ""}
            onChange={(e) => set({ probability_of_extension: e.target.value })}
          />
        </Field>
        <Field label="Remarks">
          <Input value={value.remarks ?? ""} onChange={(e) => set({ remarks: e.target.value })} />
        </Field>
      </div>
    </div>
  );
}
