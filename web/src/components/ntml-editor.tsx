import {Input, Select} from "@ois/ui";

import {
  AIRCRAFT,
  ALT_OPS,
  decodeNtml,
  DIRECTIONS,
  encodeNtml,
  KINDS,
  type Ntml,
  QUALIFIERS,
  SPEED_OPS,
} from "@/lib/ntml";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-ink-2">
      {label}
      {children}
    </label>
  );
}

/** Structured NTML restriction editor with a live raw + decoded preview (the codec is mirrored in
 *  `lib/ntml.ts`; the backend re-encodes authoritatively on save). */
export function NtmlEditor({ value, onChange }: { value: Ntml; onChange: (n: Ntml) => void }) {
  const set = (patch: Partial<Ntml>) => onChange({ ...value, ...patch });
  const kind = (value.kind || "MIT").toUpperCase();
  const showValue = kind === "MIT" || kind === "MINIT";
  const showText = kind === "TXT";

  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Field label="Element">
          <Input placeholder="JFK / EWR,LGA" value={value.element} onChange={(e) => set({ element: e.target.value })} />
        </Field>
        <Field label="Direction">
          <Select wrapperClassName="w-full" value={value.direction} onChange={(e) => set({ direction: e.target.value })}>
            {DIRECTIONS.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </Select>
        </Field>
        <Field label="Via (fix/airway)">
          <Input
            placeholder="CAMRN / J152"
            value={value.via ?? ""}
            onChange={(e) => set({ via: e.target.value || null })}
          />
        </Field>
        <Field label="Type">
          <Select wrapperClassName="w-full" value={kind} onChange={(e) => set({ kind: e.target.value })}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </Select>
        </Field>

        {showValue && (
          <Field label={kind === "MINIT" ? "Minutes" : "Miles"}>
            <Input
              type="number"
              value={value.value ?? ""}
              onChange={(e) => set({ value: e.target.value === "" ? null : Number(e.target.value) })}
            />
          </Field>
        )}
        {showText && (
          <Field label="Free text">
            <Input value={value.text ?? ""} onChange={(e) => set({ text: e.target.value || null })} />
          </Field>
        )}

        <Field label="Qualifier">
          <Select wrapperClassName="w-full" value={value.qualifier ?? ""} onChange={(e) => set({ qualifier: e.target.value || null })}>
            {QUALIFIERS.map((q) => (
              <option key={q} value={q}>
                {q || "—"}
              </option>
            ))}
          </Select>
        </Field>
        <Field label="Aircraft">
          <Select wrapperClassName="w-full" value={value.aircraft ?? ""} onChange={(e) => set({ aircraft: e.target.value || null })}>
            {AIRCRAFT.map((a) => (
              <option key={a} value={a}>
                {a || "—"}
              </option>
            ))}
          </Select>
        </Field>

        <Field label="Speed (kt)">
          <div className="flex gap-1">
            <Select
              className="font-mono"
              wrapperClassName="w-16 shrink-0"
              value={value.speed?.op ?? ""}
              onChange={(e) =>
                set({ speed: e.target.value ? { op: e.target.value, value: value.speed?.value ?? 250 } : null })
              }
            >
              <option value="">—</option>
              {SPEED_OPS.map((o) => (
                <option key={o} value={o}>
                  {o}
                </option>
              ))}
            </Select>
            {value.speed && (
              <Input
                type="number"
                className="w-20"
                value={value.speed.value}
                onChange={(e) => set({ speed: { op: value.speed!.op, value: Number(e.target.value) || 0 } })}
              />
            )}
          </div>
        </Field>
        <Field label="Altitude (FL)">
          <div className="flex gap-1">
            <Select
              className="font-mono"
              wrapperClassName="w-16 shrink-0"
              value={value.altitude?.op ?? ""}
              onChange={(e) =>
                set({ altitude: e.target.value ? { op: e.target.value, value: value.altitude?.value ?? 100 } : null })
              }
            >
              <option value="">—</option>
              {ALT_OPS.map((o) => (
                <option key={o} value={o}>
                  {o}
                </option>
              ))}
            </Select>
            {value.altitude && (
              <Input
                type="number"
                className="w-20"
                value={value.altitude.value}
                onChange={(e) => set({ altitude: { op: value.altitude!.op, value: Number(e.target.value) || 0 } })}
              />
            )}
          </div>
        </Field>

        <Field label="Condition">
          <Input
            placeholder="VOLUME / WEATHER"
            value={value.condition ?? ""}
            onChange={(e) => set({ condition: e.target.value || null })}
          />
        </Field>
        <Field label="Cond. detail">
          <Input
            placeholder="THUNDERSTORMS"
            value={value.condition_detail ?? ""}
            onChange={(e) => set({ condition_detail: e.target.value || null })}
          />
        </Field>
        <Field label="Exclude">
          <Input
            placeholder="PHL, PNE"
            value={(value.exclude ?? []).join(", ")}
            onChange={(e) =>
              set({ exclude: e.target.value.split(/[,\s]+/).map((x) => x.trim().toUpperCase()).filter(Boolean) })
            }
          />
        </Field>
      </div>

      <div className="rounded-sm border border-line bg-panel-2 p-3">
        <div className="flex gap-2">
          <span className="w-16 shrink-0 text-xs font-semibold text-ink-3">Raw</span>
          <span className="font-mono text-sm text-ink">{encodeNtml(value) || "—"}</span>
        </div>
        <div className="mt-1.5 flex gap-2">
          <span className="w-16 shrink-0 text-xs font-semibold text-ink-3">Decoded</span>
          <span className="text-sm text-ink-2">{decodeNtml(value)}</span>
        </div>
      </div>
    </div>
  );
}
