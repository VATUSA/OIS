import {useEffect, useState} from "react";
import {Input} from "@ois/ui";

import {parseLimit} from "@/lib/rate-limits";

/**
 * A credential's rate limit (#611): read-only text, or — for an admin — an input that saves on Enter
 * or blur through `parseLimit` (empty clears to the default) and otherwise snaps back.
 */
export function RateLimitCell({
  label,
  value,
  editable,
  onSave,
}: {
  label: string;
  value: number | null | undefined;
  editable: boolean;
  onSave: (perMin: number | null) => void;
}) {
  const shown = value == null ? "" : String(value);
  const [draft, setDraft] = useState(shown);
  useEffect(() => setDraft(shown), [shown]);

  if (!editable) {
    return <span className="whitespace-nowrap text-ink-2">{value == null ? "default" : `${value}/min`}</span>;
  }

  const commit = () => {
    const next = parseLimit(draft);
    if (next === undefined || next === (value ?? null)) setDraft(shown);
    else onSave(next);
  };

  return (
    <Input
      aria-label={`${label} rate limit per minute`}
      className="h-8 w-24 font-mono"
      inputMode="numeric"
      placeholder="default"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
    />
  );
}
