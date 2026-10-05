import {readdirSync, readFileSync} from "node:fs";
import {resolve} from "node:path";

import {describe, expect, it} from "vitest";

/**
 * The sector grid's colours come from tokens, never literals (#724, DESIGN.md "Tokens only"). The
 * reference it is modelled on (FSM) pins beige chrome and traffic-light hexes; none of that may land
 * here. Scans every non-test source file of the component.
 */
const LITERAL_COLOUR = /#[0-9a-f]{3,8}\b|\b(rgba?|hsla?|oklch)\(/i;
const PALETTE_CLASS =
  /\b(bg|text|border|ring|fill|stroke|from|to|via|outline|decoration|accent|caret)-(slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|black|white)(-\d{2,3})?\b/;

const dir = import.meta.dirname;
const sources = readdirSync(dir)
  .filter((f) => f.startsWith("sector-grid") && /\.tsx?$/.test(f) && !/\.test\.tsx?$/.test(f))
  .map((f) => resolve(dir, f));

const offenders = (text: string) =>
  text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .map((l) => l.replace(/\/\/.*$/, ""))
    .filter((l) => LITERAL_COLOUR.test(l) || PALETTE_CLASS.test(l));

describe("sector grid colours", () => {
  it("are tokens, not literals", () => {
    expect(sources.length).toBeGreaterThan(0);
    for (const file of sources) expect(offenders(readFileSync(file, "utf8")), file).toEqual([]);
  });

  it("would catch a literal or a palette class", () => {
    for (const bad of ['style={{background: "#EFDFCE"}}', 'className="bg-green-500"', "color: rgb(61 199 68)", 'className="text-red-600"']) {
      expect(offenders(bad), bad).toHaveLength(1);
    }
    expect(offenders('className="bg-level-ok/15 border-line-soft text-ink"')).toEqual([]);
  });
});
