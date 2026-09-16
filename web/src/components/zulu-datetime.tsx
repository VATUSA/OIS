import {useEffect, useState} from "react";

const pad = (n: number) => String(n).padStart(2, "0");

/** Unix seconds (UTC) → the date + 24h-time field values. */
function parts(unixS: number): { date: string; time: string } {
  const d = new Date(unixS * 1000);
  return {
    date: `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`,
    time: `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`,
  };
}

/**
 * A themed Zulu date + 24-hour time picker whose value is unix **seconds (UTC)**. The date uses a
 * native `<input type="date">` styled with app tokens and a `color-scheme` bound to the active theme
 * (so its calendar popup is dark-aware in every theme); the time is a hand-built `HH:MM` field so it's
 * always 24-hour and fully themed, regardless of the browser's locale.
 */
export function ZuluDateTime({
  value,
  onChange,
  className,
}: {
  value: number;
  onChange: (unixS: number) => void;
  className?: string;
}) {
  const { date, time } = parts(value);

  const commit = (dateStr: string, timeStr: string) => {
    const ms = Date.parse(`${dateStr}T${timeStr}:00Z`);
    if (!Number.isNaN(ms)) onChange(Math.floor(ms / 1000));
  };

  return (
    <div className={"flex items-center gap-1.5 " + (className ?? "")}>
      <input
        type="date"
        value={date}
        onChange={(e) => e.target.value && commit(e.target.value, time)}
        className="h-9 rounded-xs border border-line bg-panel-2 px-2 font-mono text-[13px] text-ink outline-none focus-visible:ring-2 focus-visible:ring-ring [color-scheme:light] dark:[color-scheme:dark]"
        aria-label="Date (Zulu)"
      />
      <TimeField value={time} onChange={(t) => commit(date, t)} />
      <span className="font-mono text-xs text-ink-3">Z</span>
    </div>
  );
}

/** A 24-hour `HH:MM` field. Local text while typing; commits a valid, clamped value on blur/Enter. */
function TimeField({ value, onChange }: { value: string; onChange: (time: string) => void }) {
  const [text, setText] = useState(value);
  // Re-sync when the committed value changes upstream (e.g. the date or scrubber moved it).
  useEffect(() => setText(value), [value]);

  const commit = (raw: string) => {
    const m = raw.trim().match(/^(\d{1,2}):?(\d{2})$/); // "9:05", "0905", "09:05"
    if (m) {
      const h = Number(m[1]);
      const min = Number(m[2]);
      if (h <= 23 && min <= 59) {
        const t = `${pad(h)}:${pad(min)}`;
        setText(t);
        onChange(t);
        return;
      }
    }
    setText(value); // reject — snap back to the last good value
  };

  return (
    <input
      inputMode="numeric"
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={(e) => commit(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit((e.target as HTMLInputElement).value);
      }}
      placeholder="HH:MM"
      maxLength={5}
      aria-label="Time (24-hour, Zulu)"
      className="h-9 w-[4.5rem] rounded-xs border border-line bg-panel-2 px-2 text-center font-mono text-[13px] tabular-nums text-ink placeholder:text-ink-3 outline-none focus-visible:ring-2 focus-visible:ring-ring"
    />
  );
}
