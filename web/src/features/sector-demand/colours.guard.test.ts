import {readdirSync, readFileSync, statSync} from "node:fs";
import {resolve} from "node:path";

import {describe, expect, it} from "vitest";

/**
 * The Sector Monitor's colours (#725, #794). The page's body copies vTBFM's palette, a named exception
 * in DESIGN.md (§ "Named exception: the Sector Monitor body"), and that exception is held to one file:
 * `vtbfm-palette.ts` is the only source of the page allowed a literal colour, and nothing outside the
 * monitor may import it. Every other source of the page — the rest of the feature, the limit feature
 * and the page itself — is tokens only, like the rest of OIS.
 *
 * A source scan, because the rule is an absence across files (test-quality.md § Absence needs a guard):
 * no component test can see a literal added to a file it doesn't render.
 */
const LITERAL_COLOUR = /#[0-9a-f]{3,8}\b|\b(rgba?|hsla?|oklch)\(/i;
const PALETTE_CLASS =
  /\b(bg|text|border|ring|fill|stroke|from|to|via|outline|decoration|accent|caret)-(slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|black|white)(-\d{2,3})?\b/;

const src = resolve(__dirname, "../..");
const repo = resolve(src, "../..");
/** Every non-test source under `dir`, subdirectories included, so a file added below it is covered. */
const sourcesIn = (dir: string): string[] =>
  readdirSync(resolve(src, dir)).flatMap((f) => {
    const path = resolve(src, dir, f);
    if (statSync(path).isDirectory()) return sourcesIn(`${dir}/${f}`);
    return /\.(tsx?|css)$/.test(f) && !/\.test\.tsx?$/.test(f) ? [path] : [];
  });

/** The one file the DESIGN.md exception covers. */
const EXEMPT = "features/sector-demand/vtbfm-palette.ts";

const SOURCES = [
  ...sourcesIn("features/sector-demand"),
  ...sourcesIn("features/sector-limits"),
  resolve(src, "pages/sector-monitor.tsx"),
];
const rel = (f: string) => f.slice(src.length + 1);

const stripComments = (text: string) =>
  text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    // A line comment, but not the `//` of a URL inside a string, which would hide the rest of its line.
    .map((l) => l.replace(/(^|[^:])\/\/.*$/, "$1"));

const offenders = (text: string) => stripComments(text).filter((l) => LITERAL_COLOUR.test(l) || PALETTE_CLASS.test(l));
const importsPalette = (text: string) => stripComments(text).some((l) => /from\s+["'][^"']*vtbfm-palette["']/.test(l));

describe("sector monitor colours (#725, #794)", () => {
  it("are tokens, not literals, in every source of the page but the vTBFM palette", () => {
    expect(SOURCES.map(rel)).toEqual(
      expect.arrayContaining([
        "features/sector-demand/SectorDemand.tsx",
        "features/sector-demand/MonitorTable.tsx",
        "features/sector-demand/SectorContextMenu.tsx",
        "features/sector-demand/view.ts",
        "features/sector-demand/sector-demand.ts",
        "features/sector-limits/sector-limits.ts",
        "pages/sector-monitor.tsx",
        EXEMPT,
      ]),
    );
    for (const file of SOURCES.filter((f) => rel(f) !== EXEMPT)) {
      expect(offenders(readFileSync(file, "utf8")), rel(file)).toEqual([]);
    }
  });

  it("holds the exception to the palette file, which DESIGN.md names", () => {
    // Positive control: the palette really does carry vTBFM's literals, so the exemption is doing work.
    expect(offenders(readFileSync(resolve(src, EXEMPT), "utf8")).length).toBeGreaterThan(10);
    const design = readFileSync(resolve(repo, "DESIGN.md"), "utf8");
    expect(design).toContain("Named exception: the Sector Monitor body");
    expect(design).toContain(`web/src/${EXEMPT}`);
  });

  it("lets nothing outside the monitor's own feature import the palette", () => {
    const outside = sourcesIn(".").filter((f) => !rel(f).startsWith("features/sector-demand/"));
    expect(outside.length).toBeGreaterThan(100);
    expect(outside.filter((f) => importsPalette(readFileSync(f, "utf8"))).map(rel)).toEqual([]);
  });

  it("would catch a literal, a palette class, or a stray import", () => {
    for (const bad of [
      'style={{background: "#EFDFCE"}}',
      'className="bg-green-500"',
      "color: rgb(61 199 68)",
      'className="text-red-600"',
      'className="border-yellow-300"',
    ]) {
      expect(offenders(bad), bad).toHaveLength(1);
    }
    expect(offenders('className="bg-level-over/20 border-line-soft text-ink accent-brand"')).toEqual([]);
    // A comment is prose, not a colour; a URL's `//` is not a comment.
    expect(offenders("// vTBFM's beige was #EFDFCE")).toEqual([]);
    expect(offenders('const u = "https://x.test"; const c = "#EFDFCE";')).toHaveLength(1);
    expect(importsPalette('import {C} from "@/features/sector-demand/vtbfm-palette";')).toBe(true);
    expect(importsPalette("// from \"./vtbfm-palette\"")).toBe(false);
  });
});
