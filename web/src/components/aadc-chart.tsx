import {AADC_DIMENSIONS, type AadcBucket, type AadcBucketMin, type AadcDimension} from "@/lib/aadc";
import {hhmmZulu} from "@/lib/time";

const BUCKET_OPTS: AadcBucketMin[] = [15, 30, 60];

const STATUS_COLOR: Record<string, string> = {
  airborne: "#10b981",
  ground: "#f59e0b",
  proposed: "#0ea5e9",
};

const PALETTE = [
  "#54b8e8",
  "#57d98a",
  "#f5a83d",
  "#c792ea",
  "#f07178",
  "#38bdf8",
  "#fbbf24",
  "#a78bfa",
];

const OTHER_COLOR = "#71717a";
const OTHER = "OTHER";

function breakdown(bucket: AadcBucket, dimension: AadcDimension): Record<string, number> {
  switch (dimension) {
    case "status":
      return (bucket.by_status ?? {}) as Record<string, number>;
    case "category":
      return (bucket.by_category ?? {}) as Record<string, number>;
    case "carrier":
      return (bucket.by_carrier ?? {}) as Record<string, number>;
    case "afix":
      return (bucket.by_afix ?? {}) as Record<string, number>;
  }
}

/** Every key that appears in any bucket's breakdown, ranked busiest-first (OTHER always last) —
 * a stable order so a key's color/stack position doesn't shuffle bucket to bucket. */
function rankedKeys(buckets: AadcBucket[], dimension: AadcDimension): string[] {
  const totals: Record<string, number> = {};
  for (const b of buckets) {
    for (const [k, n] of Object.entries(breakdown(b, dimension))) {
      totals[k] = (totals[k] ?? 0) + n;
    }
  }
  return Object.keys(totals)
    .filter((k) => k !== OTHER)
    .sort((a, b) => totals[b] - totals[a] || a.localeCompare(b))
    .concat(totals[OTHER] ? [OTHER] : []);
}

function colorFor(dimension: AadcDimension, key: string, index: number): string {
  if (key === OTHER) return OTHER_COLOR;
  if (dimension === "status") return STATUS_COLOR[key] ?? OTHER_COLOR;
  return PALETTE[index % PALETTE.length];
}

export function AadcChart({
  icao,
  bucketMin,
  onBucketMinChange,
  dimension,
  onDimensionChange,
  buckets,
  aar,
  isLoading,
}: {
  icao: string;
  bucketMin: AadcBucketMin;
  onBucketMinChange: (v: AadcBucketMin) => void;
  dimension: AadcDimension;
  onDimensionChange: (v: AadcDimension) => void;
  buckets: AadcBucket[];
  aar: number;
  isLoading?: boolean;
}) {
  const cap = aar > 0 ? Math.max(1, Math.round(aar / (60 / bucketMin))) : 0;
  const max = Math.max(...buckets.map((b) => b.total), cap, 1);
  const keys = rankedKeys(buckets, dimension);
  const H = 200;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {icao} — arrival demand, next {(buckets.length * bucketMin) / 60}h
        </div>
        <div className="flex items-center gap-2">
          <select
            className="h-8 rounded-md border border-input bg-background px-2 text-xs"
            value={dimension}
            onChange={(e) => onDimensionChange(e.target.value as AadcDimension)}
          >
            {AADC_DIMENSIONS.map((d) => (
              <option key={d.value} value={d.value}>
                {d.label}
              </option>
            ))}
          </select>
          <div className="flex overflow-hidden rounded-md border">
            {BUCKET_OPTS.map((b) => (
              <button
                key={b}
                type="button"
                onClick={() => onBucketMinChange(b)}
                className={
                  "px-2 py-1 text-xs font-medium transition-colors " +
                  (bucketMin === b ? "bg-primary text-primary-foreground" : "hover:bg-accent/40")
                }
              >
                {b}m
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* Bars + axis labels share one scroll container (and a shared min-w-0 chain) so a wide
          bucket count scrolls locally instead of forcing the whole page wider on a phone. */}
      <div className="min-w-0 overflow-x-auto">
        <div className="min-w-max">
          <div className="relative">
            <div className="flex items-end gap-1" style={{ height: H }}>
              {buckets.map((b, i) => {
                const barH = Math.max(2, (b.total / max) * (H - 24));
                const seg = breakdown(b, dimension);
                return (
                  <div key={i} className="flex w-10 shrink-0 flex-col items-center justify-end gap-1">
                    <span className="text-[10px] tabular-nums text-muted-foreground">
                      {b.total || ""}
                    </span>
                    <div
                      className="flex w-full flex-col-reverse overflow-hidden rounded-t bg-muted"
                      style={{ height: barH }}
                      title={`${hhmmZulu(b.start)}: ${b.total} arrivals`}
                    >
                      {keys.map((k) => {
                        const n = seg[k] ?? 0;
                        if (n === 0) return null;
                        return (
                          <div
                            key={k}
                            style={{
                              height: `${(n / b.total) * 100}%`,
                              background: colorFor(dimension, k, keys.indexOf(k)),
                            }}
                            title={`${k}: ${n}`}
                          />
                        );
                      })}
                    </div>
                  </div>
                );
              })}
            </div>
            {cap > 0 && (
              <div
                className="pointer-events-none absolute inset-x-0 border-t-2 border-dashed border-amber-400"
                style={{ bottom: 24 + (cap / max) * (H - 24) }}
              >
                <span className="absolute -top-4 right-0 text-[10px] font-medium text-amber-400">
                  AAR {aar}/hr → {cap}/bucket
                </span>
              </div>
            )}
          </div>

          <div className="flex gap-1 text-[10px] font-mono text-muted-foreground">
            {buckets.map((b, i) => (
              <span key={i} className="w-10 shrink-0 text-center">
                {hhmmZulu(b.start)}
              </span>
            ))}
          </div>
        </div>
      </div>

      <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
        {keys.map((k, i) => (
          <span key={k} className="flex items-center gap-1">
            <span
              className="inline-block size-2 rounded-sm"
              style={{ background: colorFor(dimension, k, i) }}
            />
            {k}
          </span>
        ))}
        {isLoading && <span>Loading…</span>}
      </div>
    </div>
  );
}
