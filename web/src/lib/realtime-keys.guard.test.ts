import {readdirSync, readFileSync, statSync} from "node:fs";
import {join} from "node:path";
import {describe, expect, it} from "vitest";

import {TOPIC_KEYS} from "./realtime";

/**
 * VATUSA/OIS#647: `TOPIC_KEYS` is hand-maintained against query keys defined elsewhere, and a dead
 * entry is silent — invalidating a key no hook builds simply does nothing. Every prefix in the map
 * must appear as the first element of an array literal in some other source file: an inline
 * `queryKey: [...]`, or a key factory such as `fcaKey` (`["event-fcas", eventId]`).
 */

const SRC = join(__dirname, "..");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    const isSource = /\.(ts|tsx)$/.test(name) && !/\.(test|guard\.test|dom\.test)\.tsx?$/.test(name);
    return isSource && !path.endsWith(join("lib", "realtime.ts")) ? [path] : [];
  });
}

/** The `topic: prefix` pairs whose prefix no file in `sources` builds a query key from. */
function deadKeys(map: Record<string, string[][]>, sources: string[]): string[] {
  const used = new Set<string>();
  for (const src of sources) {
    for (const m of src.matchAll(/\[\s*"([a-z0-9-]+)"\s*[,\]]/g)) used.add(m[1]);
  }
  return Object.entries(map).flatMap(([topic, keys]) =>
    keys.filter(([prefix]) => !used.has(prefix)).map(([prefix]) => `${topic}: ${prefix}`),
  );
}

const SOURCES = sourceFiles(SRC).map((f) => readFileSync(f, "utf8"));

describe("TOPIC_KEYS names only query keys that exist (#647)", () => {
  it("every topic's keys are built by some hook", () => {
    expect(SOURCES.length, "scanned the web source").toBeGreaterThan(50);
    expect(deadKeys(TOPIC_KEYS, SOURCES)).toEqual([]);
  });

  it("an event's FCA list is among them: fcaKey builds [\"event-fcas\", eventId]", () => {
    expect(TOPIC_KEYS["flow.fca"]).toContainEqual(["event-fcas"]);
    expect(SOURCES.some((s) => s.includes('["event-fcas", eventId]'))).toBe(true);
  });

  it("catches a dead entry, and a key whose only builder disappears", () => {
    expect(deadKeys({ ...TOPIC_KEYS, "made.up": [["no-such-key"]] }, SOURCES)).toEqual([
      "made.up: no-such-key",
    ]);
    const withoutFcaKey = SOURCES.map((s) => s.replaceAll('["event-fcas", eventId]', "[eventId]"));
    expect(deadKeys(TOPIC_KEYS, withoutFcaKey)).toEqual(["flow.fca: event-fcas"]);
  });
});
