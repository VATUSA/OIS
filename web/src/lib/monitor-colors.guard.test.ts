import {readdirSync, readFileSync, statSync} from "node:fs";
import {join} from "node:path";
import {describe, expect, it} from "vitest";

import {ALERT_LEVEL, alertTextClass, alertToken} from "./monitor-alert";

/**
 * Monitor colours come from tokens, never literals (#600). DESIGN.md: the alert states are the
 * load-level semantics, and domain colours are "never re-declared as hex". This scans every Monitor
 * source file — any path naming the monitor — so the page #601 adds is covered as it lands.
 *
 * Limit: it matches by path. Monitor code in a file whose path doesn't say "monitor" slips through.
 */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.(tsx?|css)$/.test(entry) && !/\.test\.tsx?$/.test(entry) ? [path] : [];
  });
}

/**
 * Whether `path` is Monitor code, judged on its path *within* `web/src`, never on where the checkout
 * lives: a worktree under `…/monitor-page/` would otherwise make every file look like one (#709).
 */
function isMonitorFile(path: string): boolean {
  return /monitor/i.test(path.replace(/.*\/web\/src\//, ""));
}

function withoutComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
}

const LITERAL_COLOUR = /#[0-9a-f]{3,8}\b|\b(rgba?|hsla?)\(/i;

describe("Monitor colours", () => {
  it("are tokens, not literals", () => {
    const files = sourceFiles(new URL("..", import.meta.url).pathname).filter(isMonitorFile);
    expect(files.length).toBeGreaterThan(0);
    const offenders = files
      .filter((path) => LITERAL_COLOUR.test(withoutComments(readFileSync(path, "utf8"))))
      .map((path) => path.replace(/.*\/web\/src\//, "web/src/"));
    expect(offenders).toEqual([]);
  });

  it("selects files by their path within web/src, not the checkout's (#709)", () => {
    const checkout = "/home/dev/ois-wt/feat/601/monitor-page/web/src/";
    expect(isMonitorFile(`${checkout}lib/facility-map/palette.ts`)).toBe(false);
    expect(isMonitorFile(`${checkout}pages/flow/monitor.tsx`)).toBe(true);
    expect(isMonitorFile("/home/dev/OIS/web/src/lib/monitor-alert.ts")).toBe(true);
  });

  it("would notice one — the pattern matches the shapes it forbids", () => {
    for (const bad of ['color: "#fb6b6b"', "fill: '#f00'", "rgba(251, 107, 107, 0.13)", "hsl(0 90% 70%)"]) {
      expect(LITERAL_COLOUR.test(bad)).toBe(true);
    }
    expect(LITERAL_COLOUR.test("text-level-over")).toBe(false);
    expect(LITERAL_COLOUR.test(withoutComments("// was #fb6b6b in vTBFM"))).toBe(false);
  });

  it("map each state to its load-level token", () => {
    expect(ALERT_LEVEL).toEqual({green: "level-ok", amber: "level-watch", red: "level-over"});
    expect(alertTextClass).toEqual({green: "text-level-ok", amber: "text-level-watch", red: "text-level-over"});
    expect(alertToken).toEqual({green: "--level-ok", amber: "--level-watch", red: "--level-over"});
  });
});
