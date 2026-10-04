import {readdirSync, readFileSync, statSync} from "node:fs";
import {join} from "node:path";
import {describe, expect, it} from "vitest";

/**
 * Changelog screenshots are bundled into the web app (#665), and entries are never pruned, so nothing
 * else stops the bundle growing by every shot ever shipped. `changelogProblems` bounds how many shots
 * an entry carries and how long they stay; this bounds what they weigh on disk.
 *
 * It also pins the desktop app's `img-src`. Shots load there only because they're served from our own
 * origin; widening the CSP to reach a remote image is the move #429 refused.
 */
const KB = 1024;
const MAX_FILE_BYTES = 300 * KB;
const MAX_TOTAL_BYTES = 1536 * KB;

function assetProblems(files: { name: string; bytes: number }[]): string[] {
  const problems = files
    .filter((f) => f.bytes > MAX_FILE_BYTES)
    .map((f) => `${f.name} is ${Math.round(f.bytes / KB)} KB (max ${MAX_FILE_BYTES / KB} KB)`);
  const total = files.reduce((sum, f) => sum + f.bytes, 0);
  if (total > MAX_TOTAL_BYTES) {
    problems.push(`changelog assets total ${Math.round(total / KB)} KB (max ${MAX_TOTAL_BYTES / KB} KB)`);
  }
  return problems;
}

function assetFiles(dir: string): { name: string; bytes: number }[] {
  return readdirSync(dir).flatMap((entry) => {
    if (entry.startsWith(".")) return [];
    const path = join(dir, entry);
    const stat = statSync(path);
    return stat.isDirectory()
      ? assetFiles(path).map((f) => ({ ...f, name: `${entry}/${f.name}` }))
      : [{ name: entry, bytes: stat.size }];
  });
}

describe("changelog screenshot weight", () => {
  it("keeps every file and the total within budget", () => {
    const dir = new URL("../assets/changelog", import.meta.url).pathname;
    expect(assetProblems(assetFiles(dir))).toEqual([]);
  });

  it("would notice a file or a total over budget — and nothing at the limit", () => {
    expect(assetProblems([{ name: "a.png", bytes: MAX_FILE_BYTES }])).toEqual([]);
    expect(assetProblems([{ name: "a.png", bytes: MAX_FILE_BYTES + 1 }])).toHaveLength(1);
    const atTotal = Array.from({ length: 6 }, (_, i) => ({ name: `${i}.png`, bytes: MAX_TOTAL_BYTES / 6 }));
    expect(assetProblems(atTotal)).toEqual([]);
    expect(assetProblems([...atTotal, { name: "x.png", bytes: 1 }])).toHaveLength(1);
  });
});

describe("desktop image policy", () => {
  it("still serves images from our own origin only (plus the map tiles)", () => {
    const conf = JSON.parse(
      readFileSync(new URL("../../../desktop/src-tauri/tauri.conf.json", import.meta.url), "utf8"),
    );
    expect(conf.app.security.csp["img-src"]).toBe("'self' data: blob: https://*.cartocdn.com");
  });
});
