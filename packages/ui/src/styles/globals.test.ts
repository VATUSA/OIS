import {readFile} from "node:fs/promises";
import {resolve} from "node:path";

import {beforeAll, describe, expect, it} from "vitest";

import {contrastRatio} from "../lib/colour";

/**
 * The stylesheet is the system's single source of tokens (DESIGN.md), and a token that is *used* but
 * never *defined* fails in the quietest way CSS has: `var(--x)` with no fallback is invalid at
 * computed-value time, so the declaration is dropped and the property falls back to its initial
 * value — no build error, no console warning, no failing test.
 *
 * #419 shipped `border-radius: var(--pill)` against a `--pill` that existed only as prose in
 * DESIGN.md's token table. It computed to `0`, and the desktop app's traffic lights rendered as three
 * coloured squares on Windows and Linux. Nothing caught it, because jsdom has no CSS engine and the
 * only reviewer of a stylesheet is whoever reads it (#419 review).
 */
let css = "";
beforeAll(async () => {
  css = await readFile(resolve(import.meta.dirname, "globals.css"), "utf8");
});

/** Definitions and uses, with comments stripped so a commented-out token doesn't count as either. */
function tokens(source: string) {
  const bare = source.replace(/\/\*[\s\S]*?\*\//g, "");
  const defined = new Set([...bare.matchAll(/(--[a-zA-Z0-9-]+)\s*:/g)].map((m) => m[1]));
  // Only uses with no fallback: `var(--x, 4px)` degrades to 4px on its own and is a deliberate choice.
  const used = new Set(
    [...bare.matchAll(/var\(\s*(--[a-zA-Z0-9-]+)\s*(,)?/g)].filter((m) => !m[2]).map((m) => m[1]),
  );
  return {defined, used};
}

describe("the token stylesheet", () => {
  it("defines every token it uses without a fallback", () => {
    const {defined, used} = tokens(css);
    const missing = [...used].filter((t) => !defined.has(t)).sort();

    expect(missing).toEqual([]);
  });

  /** The regex above has to actually see an undefined token, or the test above is decoration. */
  it("would catch one that is only declared in prose", () => {
    const {defined, used} = tokens(":root { --a: 1px } .x { border-radius: var(--ghost) }");

    expect(defined.has("--a")).toBe(true);
    expect([...used].filter((t) => !defined.has(t))).toEqual(["--ghost"]);
  });

  it("does not flag a use that carries its own fallback", () => {
    const {used} = tokens(".x { gap: var(--ghost, 4px) }");

    expect([...used]).toEqual([]);
  });

  /**
   * The traffic lights are the reason this file exists, so pin the two things about them that a
   * stylesheet-only change can silently break: the pill radius that makes them round, and the fact
   * that they carry their own chrome hues rather than borrowing a semantic status token.
   */
  it("keeps the traffic lights round and on their own hues", () => {
    expect(css).toMatch(/--pill:\s*999px/);
    for (const token of ["--traffic-close", "--traffic-minimize", "--traffic-zoom", "--traffic-blurred"]) {
      expect(css).toMatch(new RegExp(`${token}:\\s*#`));
    }
    expect(css).toMatch(/\.traffic-light\s*\{[^}]*border-radius:\s*var\(--pill\)/);
    // A flex item with a definite width still shrinks when its row is too narrow, and it shrinks to
    // the 8px glyph inside it — the dots became ellipses rather than overflowing where anyone would
    // see them (#419 review). jsdom has no layout, so this is pinned in the source.
    expect(css).toMatch(/\.traffic-light\s*\{[^}]*flex:\s*0 0 12px/);
  });
});

/**
 * The load-level colours carry a load's meaning wherever one is drawn (runway bins, GDP, delays; #724),
 * so each must stand out from the ground it is drawn on in both themes: 3:1, WCAG's floor for a
 * non-text indicator. This is about the colour signal, not text. `--level-*` alias
 * success/warning/danger.
 */
describe("load-level colours (#724)", () => {
  const block = (selector: string) => {
    const at = css.indexOf(`${selector} {`);
    return css.slice(at, css.indexOf("}", at));
  };
  const value = (decls: string, name: string) => decls.match(new RegExp(`--${name}:\\s*(#[0-9a-f]{6})`, "i"))?.[1];

  for (const [theme, selector] of [["light", ":root"], ["dark", ".dark"]] as const) {
    it(`stand out from the ${theme} ground`, () => {
      const decls = block(selector);
      const ground = value(decls, "ground")!;
      expect(ground, `${theme} --ground`).toBeDefined();
      for (const level of ["success", "warning", "danger"]) {
        const hex = value(decls, level);
        expect(hex, `${theme} --${level}`).toBeDefined();
        expect(contrastRatio(hex!, ground), `${theme} --${level} on ${ground}`).toBeGreaterThanOrEqual(3);
      }
    });
  }
});
