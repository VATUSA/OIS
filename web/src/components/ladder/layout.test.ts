import {describe, expect, it} from "vitest";

import {bucketByTime, fitHorizontal, placeSlots} from "./layout";

interface Item {
  min: number;
  time: string;
  id: string;
}

const item = (id: string, min: number, time: string): Item => ({ id, min, time });

describe("bucketByTime", () => {
  it("keeps distinct times as separate one-member buckets", () => {
    const items = [item("a", 0, "t0"), item("b", 5, "t1"), item("c", 10, "t2")];
    expect(bucketByTime(items)).toEqual([[items[0]], [items[1]], [items[2]]]);
  });

  it("groups adjacent items sharing the exact same time", () => {
    const items = [item("a", 5, "t0"), item("b", 5, "t0"), item("c", 10, "t1")];
    const buckets = bucketByTime(items);
    expect(buckets).toHaveLength(2);
    expect(buckets[0].map((i) => i.id)).toEqual(["a", "b"]);
    expect(buckets[1].map((i) => i.id)).toEqual(["c"]);
  });

  it("handles an empty list", () => {
    expect(bucketByTime([])).toEqual([]);
  });
});

describe("placeSlots", () => {
  const yOf = (min: number) => 100 - min; // simple linear mapping for the test
  const base = { yOf, rowGap: 20, minGap: 5, height: 100, pad: 2 };

  it("spaces sparse slots at their natural Y (rowGap has no effect)", () => {
    const buckets = [[item("a", 0, "t0")], [item("b", 50, "t1")]];
    const placed = placeSlots(buckets, base);
    expect(placed[0].y).toBe(100);
    expect(placed[1].y).toBe(50);
  });

  it("pushes overlapping slots apart by rowGap under normal density", () => {
    const buckets = [[item("a", 0, "t0")], [item("b", 5, "t1")], [item("c", 8, "t2")]];
    const placed = placeSlots(buckets, base);
    // First slot sits at its natural Y; each subsequent slot is at most rowGap above the last.
    expect(placed[0].y).toBe(100);
    expect(placed[1].y).toBeLessThanOrEqual(placed[0].y - base.rowGap + 1e-9);
    expect(placed[2].y).toBeLessThanOrEqual(placed[1].y - base.rowGap + 1e-9);
  });

  it("shrinks the gap below rowGap under density, while it still all fits", () => {
    // 15 buckets in a 100px-tall, 2px-pad ladder: the natural per-slot gap (98/14 ≈ 7) sits
    // between minGap and rowGap, so every consecutive gap should land there uniformly.
    const buckets = Array.from({ length: 15 }, (_, i) => [item(`i${i}`, i, `t${i}`)]);
    const placed = placeSlots(buckets, base);
    for (let i = 1; i < placed.length; i++) {
      const actualGap = placed[i - 1].y - placed[i].y;
      expect(actualGap).toBeGreaterThanOrEqual(base.minGap - 1e-9);
      expect(actualGap).toBeLessThanOrEqual(base.rowGap + 1e-9);
    }
  });

  it("falls back to compressing past minGap when even minGap spacing can't fit everyone", () => {
    // 30 buckets can't possibly all sit >= minGap apart in a 98px span (that only fits ~20) — the
    // pad clamp compresses the tail further rather than pushing slots off the top. This matches the
    // documented fallback ("down to MIN_GAP, after which very dense arrivals overlap").
    const buckets = Array.from({ length: 30 }, (_, i) => [item(`i${i}`, i, `t${i}`)]);
    const placed = placeSlots(buckets, base);
    const gaps = placed.slice(1).map((p, i) => placed[i].y - p.y);
    expect(Math.min(...gaps)).toBeLessThan(base.minGap);
  });

  it("keeps every slot at or above pad", () => {
    const buckets = Array.from({ length: 30 }, (_, i) => [item(`i${i}`, i, `t${i}`)]);
    const placed = placeSlots(buckets, base);
    for (const p of placed) expect(p.y).toBeGreaterThanOrEqual(base.pad - 1e-9);
  });

  it("treats a same-time bucket as one slot regardless of member count", () => {
    const twoAtOnce = [item("a", 5, "t0"), item("b", 5, "t0")];
    const withBucket = placeSlots([[item("x", 0, "t-1")], twoAtOnce, [item("y", 10, "t1")]], base);
    const withoutBucket = placeSlots(
      [[item("x", 0, "t-1")], [item("a", 5, "t0")], [item("y", 10, "t1")]],
      base,
    );
    // Same slot count either way (3), and identical Y placement — the bucket's two members don't
    // add a second slot.
    expect(withBucket).toHaveLength(3);
    expect(withBucket[1].y).toBe(withoutBucket[1].y);
    expect(withBucket[1].bucket).toHaveLength(2);
  });
});

describe("fitHorizontal", () => {
  const widthOf = (n: number) => n;

  it("shows every member when they all fit", () => {
    const { shown, overflowCount } = fitHorizontal([10, 10, 10], widthOf, 2, 100);
    expect(shown).toEqual([10, 10, 10]);
    expect(overflowCount).toBe(0);
  });

  it("collapses whatever doesn't fit into an overflow count", () => {
    const { shown, overflowCount } = fitHorizontal([40, 40, 40, 40], widthOf, 2, 90);
    // 40 + (2+40) = 82 fits; + (2+40) = 124 doesn't.
    expect(shown).toEqual([40, 40]);
    expect(overflowCount).toBe(2);
  });

  it("always shows at least one member, even if it alone exceeds the available width", () => {
    const { shown, overflowCount } = fitHorizontal([500, 10], widthOf, 2, 50);
    expect(shown).toEqual([500]);
    expect(overflowCount).toBe(1);
  });

  it("is a no-op for a single-member bucket regardless of width", () => {
    expect(fitHorizontal([500], widthOf, 2, 0)).toEqual({ shown: [500], overflowCount: 0 });
  });

  it("handles an empty list", () => {
    expect(fitHorizontal([], widthOf, 2, 100)).toEqual({ shown: [], overflowCount: 0 });
  });
});

// VATUSA/OIS#557: the TGUI ladder places each of its rails with these same primitives, at its own
// dimensions (60 min × 6 px, rows of 18 px, a 10 px floor, 8 px top pad). The invariants the classic
// ladder earned (#62/#64/#81) must hold for a rail too.
describe("a TGUI rail (#557 AC4)", () => {
  const H = 60 * 6;
  const yOf = (min: number) => H - (Math.max(0, Math.min(min, 60)) / 60) * H;
  const rail = { yOf, rowGap: 18, minGap: 10, height: H, pad: 8 };

  it("never stretches under density: 40 slots stay inside [pad, H], NOW stays pinned at the bottom", () => {
    const buckets = Array.from({ length: 40 }, (_, i) => [item(`a${i}`, i * 0.1, `t${i}`)]);
    const placed = placeSlots(buckets, rail);
    expect(placed[0].y).toBe(H);
    for (const { y } of placed) {
      expect(y).toBeGreaterThanOrEqual(rail.pad);
      expect(y).toBeLessThanOrEqual(H);
    }
  });

  it("pushes a packed slot up, never down — each sits at or above its true time", () => {
    const buckets = [0, 0.2, 0.4, 0.6, 30].map((m, i) => [item(`a${i}`, m, `t${i}`)]);
    const placed = placeSlots(buckets, rail);
    placed.forEach(({ y, bucket }, i) => {
      expect(y).toBeLessThanOrEqual(yOf(bucket[0].min));
      if (i > 0) expect(y).toBeLessThan(placed[i - 1].y);
    });
    // A slot with room is not moved at all.
    expect(placed[4].y).toBe(yOf(30));
  });

  it("gives six same-time aircraft one slot, and fits them to the rail's width with an overflow", () => {
    const same = Array.from({ length: 6 }, (_, i) => item(`a${i}`, 20, "t"));
    const buckets = bucketByTime(same);
    expect(buckets).toHaveLength(1);
    expect(placeSlots(buckets, rail)).toHaveLength(1);

    const { shown, overflowCount } = fitHorizontal(buckets[0], () => 50, 4, 85);
    expect(shown).toHaveLength(1);
    expect(overflowCount).toBe(5);
  });
});
