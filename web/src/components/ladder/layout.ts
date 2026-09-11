// Pure geometry shared by the FCA metering ladder (pages/fca/detail.tsx) and the airport arrival
// ladder (pages/airport.tsx, also the `airport-ladder` dashboard widget): bucket flights sharing an
// exact crossing/arrival time into one vertical slot, place slots with a fixed axis + adaptive gap
// that shrinks under density instead of stretching the timeline, and fit a bucket's members
// horizontally within the ladder's actual rendered width, collapsing whatever doesn't fit into an
// overflow count for a "+N" cluster.

export interface TimedItem {
  /** Minutes from now (can be negative just after crossing). */
  min: number;
  /** Exact instant identity — items sharing this value bucket together. */
  time: string;
}

/** Group pre-sorted (by `min`, ascending) items into buckets of identical `time`. Buckets preserve
 * their members' relative order; a lone item is its own one-member bucket. */
export function bucketByTime<T extends TimedItem>(items: T[]): T[][] {
  const buckets: T[][] = [];
  for (const item of items) {
    const last = buckets[buckets.length - 1];
    if (last && last[0].time === item.time) last.push(item);
    else buckets.push([item]);
  }
  return buckets;
}

export interface PlaceOptions {
  yOf: (min: number) => number;
  rowGap: number;
  minGap: number;
  height: number;
  pad: number;
}

export interface PlacedSlot<T> {
  y: number;
  bucket: T[];
}

/** Fixed-axis declutter: NOW stays pinned, the container never grows. Adjacent slots are pushed
 * apart by `rowGap` normally; under density the gap shrinks (down to `minGap`) so everything still
 * fits between `pad` and the top instead of stretching the timeline. Operates on buckets — one slot
 * per bucket — so a same-time cluster only ever claims a single slot's space, however many members
 * it has. */
export function placeSlots<T extends TimedItem>(
  buckets: T[][],
  { yOf, rowGap, minGap, height, pad }: PlaceOptions,
): PlacedSlot<T>[] {
  const gap = Math.max(minGap, Math.min(rowGap, (height - pad) / Math.max(buckets.length - 1, 1)));
  let lastY = height + gap;
  return buckets.map((bucket) => {
    const y = Math.max(pad, Math.min(yOf(bucket[0].min), lastY - gap));
    lastY = y;
    return { y, bucket };
  });
}

export interface FitResult<T> {
  shown: T[];
  overflowCount: number;
}

/** Greedy left-to-right fit: keep adding members while they still fit `available` width (each
 * costing `widthOf(member)` plus one `gap` after the first); anything left over becomes the
 * overflow count for a "+N" cluster. Always shows at least one member — an empty slot would be
 * worse than one overflowing tag. */
export function fitHorizontal<T>(
  members: T[],
  widthOf: (t: T) => number,
  gap: number,
  available: number,
): FitResult<T> {
  if (members.length <= 1) return { shown: members, overflowCount: 0 };
  const shown: T[] = [];
  let used = 0;
  for (const m of members) {
    const w = widthOf(m) + (shown.length > 0 ? gap : 0);
    if (shown.length > 0 && used + w > available) break;
    shown.push(m);
    used += w;
  }
  if (shown.length === 0) shown.push(members[0]);
  return { shown, overflowCount: members.length - shown.length };
}
