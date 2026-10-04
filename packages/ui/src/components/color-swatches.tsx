import * as React from "react";

import {meetsGroundFloor, normalizeHex} from "../lib/colour";
import {cn} from "../lib/utils";

export interface Swatch {
  hex: string;
  label: string;
}

/**
 * A row of named colour swatches for a persisted, user-chosen colour (DESIGN.md § "User-chosen domain
 * colour"). Each swatch is labelled by name, not hex. A saved value outside the set still shows, as its
 * own swatch labelled with its hex. With `allowCustom`, a native colour input adds any `#rrggbb` — but
 * not one too dark to see on the dark ground, which is refused here as the server would refuse it.
 */
export function ColorSwatches({
  swatches,
  value,
  onChange,
  allowCustom,
  label,
  labelFor = (hex) => hex,
}: {
  swatches: Swatch[];
  value: string;
  onChange: (hex: string) => void;
  allowCustom?: boolean;
  /** A heading above the row; none when omitted. */
  label?: string;
  /** Names a saved value outside `swatches`; its hex by default. */
  labelFor?: (hex: string) => string;
}) {
  const [refused, setRefused] = React.useState<string | null>(null);
  const current = value.toLowerCase();
  const known = !value || swatches.some((s) => s.hex.toLowerCase() === current);
  const all = known ? swatches : [...swatches, { hex: value, label: labelFor(value) }];

  return (
    <div className="flex flex-col gap-1">
      {label && (
        <span className="text-xs font-semibold uppercase tracking-wide text-ink-3">{label}</span>
      )}
      <div className="flex flex-wrap items-center gap-1.5">
        {all.map((s) => {
          const on = s.hex.toLowerCase() === current;
          return (
            <button
              key={s.hex}
              type="button"
              title={s.label}
              aria-label={s.label}
              aria-pressed={on}
              onClick={() => {
                setRefused(null);
                onChange(s.hex);
              }}
              className={cn("size-6 rounded-full", on && "ring-2 ring-ring ring-offset-2 ring-offset-panel")}
              // User data, not chrome: the swatch is the colour it stands for.
              style={{ background: s.hex }}
            />
          );
        })}
        {allowCustom && (
          <input
            type="color"
            aria-label="Custom colour"
            title="Custom colour"
            value={normalizeHex(value) ?? "#000000"}
            onChange={(e) => {
              const hex = normalizeHex(e.target.value);
              if (hex && meetsGroundFloor(hex)) {
                setRefused(null);
                onChange(hex);
              } else {
                setRefused(e.target.value);
              }
            }}
            className="h-6 w-8 cursor-pointer rounded-xs border border-line bg-transparent"
          />
        )}
      </div>
      {refused && (
        <p className="text-xs text-danger">
          {refused} is too dark to see on the map. Pick a lighter colour.
        </p>
      )}
    </div>
  );
}
