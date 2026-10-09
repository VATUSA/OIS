import {readdirSync, readFileSync, statSync} from "node:fs";
import {resolve} from "node:path";

import {describe, expect, it} from "vitest";

/**
 * No hardcoded colour anywhere in the sector-demand feature (#720, #725; DESIGN.md "Tokens only").
 * The same shape as `packages/ui/src/components/sector-grid.guard.test.ts`, over the page's own paths:
 * the demand and limit features and the Operations page. The FSM reference pins beige chrome and
 * traffic-light hexes; none of that may land here, and a level reads only through `--level-*`.
 */
const LITERAL_COLOUR = /#[0-9a-f]{3,8}\b|\b(rgba?|hsla?|oklch)\(/i;
const PALETTE_CLASS =
  /\b(bg|text|border|ring|fill|stroke|from|to|via|outline|decoration|accent|caret)-(slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|black|white)(-\d{2,3})?\b/;

const src = resolve(__dirname, "../..");
/** Every non-test source under `dir`, subdirectories included, so a file added below it is covered. */
const sourcesIn = (dir: string): string[] =>
  readdirSync(resolve(src, dir)).flatMap((f) => {
    const path = resolve(src, dir, f);
    if (statSync(path).isDirectory()) return sourcesIn(`${dir}/${f}`);
    return /\.(tsx?|css)$/.test(f) && !/\.test\.tsx?$/.test(f) ? [path] : [];
  });

const SOURCES = [
  ...sourcesIn("features/sector-demand"),
  ...sourcesIn("features/sector-limits"),
  resolve(src, "pages/sector-monitor.tsx"),
];

const offenders = (text: string) =>
  text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    // A line comment, but not the `//` of a URL inside a string, which would hide the rest of its line.
    .map((l) => l.replace(/(^|[^:])\/\/.*$/, "$1"))
    .filter((l) => LITERAL_COLOUR.test(l) || PALETTE_CLASS.test(l));

describe("sector demand colours (#725)", () => {
  it("are tokens, not literals, in every source of the page", () => {
    const names = SOURCES.map((f) => f.slice(src.length + 1));
    expect(names).toEqual(
      expect.arrayContaining([
        "features/sector-demand/SectorDemand.tsx",
        "features/sector-demand/view.ts",
        "features/sector-demand/sector-demand.ts",
        "features/sector-limits/sector-limits.ts",
        "pages/sector-monitor.tsx",
      ]),
    );
    for (const file of SOURCES) expect(offenders(readFileSync(file, "utf8")), file).toEqual([]);
  });

  it("would catch a literal or a palette class", () => {
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
    expect(offenders("// FSM's beige was #EFDFCE")).toEqual([]);
    expect(offenders('const u = "https://x.test"; const c = "#EFDFCE";')).toHaveLength(1);
  });
});
