import {readFileSync} from "node:fs";
import {resolve} from "node:path";

import * as React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {Button, buttonVariants} from "./button";

/** Whitespace-split class tokens, so `border-line-soft` or `hover:border-line` never pass for `border-line`. */
const tokens = (classes: string) => classes.split(/\s+/).filter(Boolean);

/** The first `selector {` block, from the selector up to its closing brace, matched brace for brace. */
function block(css: string, selector: string): string {
  const start = css.indexOf(`${selector} {`);
  expect(start, `${selector} block in globals.css`).toBeGreaterThan(-1);
  let depth = 0;
  for (let i = css.indexOf("{", start); i < css.length; i++) {
    if (css[i] === "{") depth++;
    if (css[i] === "}" && --depth === 0) return css.slice(start, i);
  }
  throw new Error(`${selector} block in globals.css is not closed`);
}

// DESIGN.md "Buttons": secondary = `--panel-2` pill with a hairline (#950).
describe("secondary button variant", () => {
  it("is a pill with a border and the line hairline", () => {
    const classes = tokens(buttonVariants({ variant: "secondary" }));
    expect(classes).toContain("border");
    expect(classes).toContain("border-line");
    expect(classes).toContain("rounded-full");
  });

  it("fills with the secondary token and no other background", () => {
    const backgrounds = tokens(buttonVariants({ variant: "secondary" })).filter((c) => c.startsWith("bg-"));
    expect(backgrounds).toEqual(["bg-secondary"]);
  });

  it("maps the secondary fill to panel-2 and the hairline to line in both themes", () => {
    const css = readFileSync(resolve(import.meta.dirname, "../styles/globals.css"), "utf8");
    expect(block(css, ":root")).toMatch(/--secondary:\s*var\(--panel-2\);/);
    expect(block(css, ".dark")).toMatch(/--secondary:\s*var\(--panel-2\);/);
    const theme = block(css, "@theme inline");
    expect(theme).toMatch(/--color-secondary:\s*var\(--secondary\);/);
    expect(theme).toMatch(/--color-line:\s*var\(--line\);/);
  });

  it("keeps the hairline when a buttonVariants call site passes a className", () => {
    const classes = tokens(buttonVariants({ variant: "secondary", size: "sm", className: "h-7" }));
    expect(classes).toContain("border");
    expect(classes).toContain("border-line");
    expect(classes).toContain("h-7");
  });

  it("renders the hairline on a Button with variant secondary and a className", () => {
    const html = renderToStaticMarkup(
      <Button variant="secondary" className="h-7 px-2">
        Retry
      </Button>,
    );
    const classes = tokens(/class="([^"]*)"/.exec(html)?.[1] ?? "");
    for (const c of ["border", "border-line", "bg-secondary", "h-7", "px-2"]) expect(classes).toContain(c);
  });
});
