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

function withoutComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
}

const LITERAL_COLOUR = /#[0-9a-f]{3,8}\b|\b(rgba?|hsla?)\(/i;

describe("Monitor colours", () => {
  it("are tokens, not literals", () => {
    const files = sourceFiles(new URL("..", import.meta.url).pathname).filter((p) => /monitor/i.test(p));
    expect(files.length).toBeGreaterThan(0);
    const offenders = files
      .filter((path) => LITERAL_COLOUR.test(withoutComments(readFileSync(path, "utf8"))))
      .map((path) => path.replace(/.*\/web\/src\//, "web/src/"));
    expect(offenders).toEqual([]);
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
