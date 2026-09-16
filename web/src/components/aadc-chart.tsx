import {Select, SegmentedControl, StackedBars, type StackKey} from "@ois/ui";

import {AADC_DIMENSIONS, type AadcBucket, type AadcBucketMin, type AadcDimension} from "@/lib/aadc";
import {hhmmZulu} from "@/lib/time";

const BUCKET_OPTS = [15, 30, 60].map((b) => ({ value: String(b), label: `${b}m` }));

/** Status keys use the flight-state tokens; every other breakdown cycles the series tokens. */
const STATUS_TOKEN: Record<string, string> = {
  airborne: "flight-airborne",
  ground: "flight-ground",
  proposed: "flight-proposed",
};

const OTHER_TOKEN = "flight-arrived";
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
  if (key === OTHER) return OTHER_TOKEN;
  if (dimension === "status") return STATUS_TOKEN[key] ?? OTHER_TOKEN;
  return `series-${(index % 8) + 1}`;
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
  const keys: StackKey[] = rankedKeys(buckets, dimension).map((k, i) => ({
    key: k,
    label: k,
    color: colorFor(dimension, k, i),
  }));
  const data = buckets.map((b) => ({ category: b.start, parts: breakdown(b, dimension) }));

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="text-xs font-semibold uppercase tracking-wide text-ink-2">
          <span className="font-mono">{icao}</span> — arrival demand, next {(buckets.length * bucketMin) / 60}h
          {cap > 0 && <span className="ml-2 font-mono normal-case text-warning">AAR {aar}/hr → {cap}/bucket</span>}
        </div>
        <div className="flex items-center gap-2">
          <Select size="sm" value={dimension} onChange={(e) => onDimensionChange(e.target.value as AadcDimension)}>
            {AADC_DIMENSIONS.map((d) => (
              <option key={d.value} value={d.value}>
                {d.label}
              </option>
            ))}
          </Select>
          <SegmentedControl
            aria-label="Bucket size"
            size="sm"
            value={String(bucketMin)}
            onChange={(v) => onBucketMinChange(Number(v) as AadcBucketMin)}
            options={BUCKET_OPTS}
          />
        </div>
      </div>

      {/* A wide bucket count scrolls locally instead of forcing the page wider on a phone. */}
      <div className="min-w-0 overflow-x-auto">
        <div style={{ minWidth: buckets.length * 28 }}>
          <StackedBars
            label={`${icao} arrival demand`}
            data={data}
            keys={keys}
            cap={cap > 0 ? cap : undefined}
            categoryFormat={hhmmZulu}
            height={220}
          />
        </div>
      </div>

      <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-ink-2">
        {keys.map((k) => (
          <span key={k.key} className="flex items-center gap-1">
            <span className="inline-block size-2 rounded-full" style={{ background: `var(--${k.color})` }} />
            {k.label}
          </span>
        ))}
        {isLoading && <span>Loading…</span>}
      </div>
    </div>
  );
}
