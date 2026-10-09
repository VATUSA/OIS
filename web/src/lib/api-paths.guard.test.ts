import {readdirSync, readFileSync} from "node:fs";
import {join, relative} from "node:path";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";

/**
 * No browser-navigable URL is built from a bare `/api/` path (VATUSA/OIS#738 AC4).
 *
 * In production the web app and the API are different hosts. A root-relative `/api/…` resolves
 * against the page's own origin, where nginx answers with the SPA shell, so the user lands on the
 * app's not-found page. It works in dev, where Vite proxies `/api`, so only a source scan sees it.
 * The download buttons shipped this way through a helper (`downloadHref = (p) => \`/api/…\``) that
 * was then used as an href, so checking `href=` alone would not have caught them.
 *
 * The rule: a literal starting with `/api/` may only be the first argument of a call, which is the
 * typed client (`ois.GET("/api/…")`, resolved against `API_BASE`) or `new URL("/api/…", base)`, or a
 * type cast (`as "/api/…"`). A navigation call is not exempt. Anything else is a hand-built URL: build
 * it from `API_BASE` (`@/lib/api`) the way `pages/download.tsx` does.
 */
const SRC = fileURLToPath(new URL("..", import.meta.url));

function sources(dir: string): string[] {
  return readdirSync(dir, {withFileTypes: true}).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sources(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [path] : [];
  });
}

/**
 * Comments blanked out, newlines kept, so a match's line number is still the real one.
 *
 * One left-to-right pass that matches string and template literals first and keeps them, so a `/*` or
 * `//` inside a string never starts a comment, and a `//` comment runs to its line's end, `/*` and all.
 */
const LITERAL_OR_COMMENT =
  /("(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`)|\/\/[^\n]*|\/\*[\s\S]*?\*\//g;
function stripComments(text: string): string {
  return text.replace(LITERAL_OR_COMMENT, (m, literal?: string) =>
    literal ? m : m.replace(/[^\n]/g, " "),
  );
}

const BARE_API_LITERAL = /["'`]\/api\//g;
const NAVIGATION_CALL = /(?:\bopen|\blocation\.(?:assign|replace))\(\s*$/;
const ALLOWED_BEFORE = /(?:\(|\bas)\s*$/;

/** `file:line` for every bare `/api/` literal that is not a call argument or a type cast. */
function offenders(text: string, file: string): string[] {
  const code = stripComments(text);
  return [...code.matchAll(BARE_API_LITERAL)]
    .filter(({index}) => {
      const before = code.slice(Math.max(0, index - 80), index);
      return NAVIGATION_CALL.test(before) || !ALLOWED_BEFORE.test(before);
    })
    .map(({index}) => `${file}:${code.slice(0, index).split("\n").length}`);
}

describe("browser-navigable API URLs go through API_BASE (#738 AC4)", () => {
  it("builds no href, location or helper from a bare /api/ path in web/src", () => {
    const files = sources(SRC);
    // An empty scan finds nothing, so it would pass on any filter or path mistake: pin that it reads
    // the page this guard exists for and the module the rule points people at.
    expect(files.map((path) => relative(SRC, path))).toEqual(
      expect.arrayContaining([join("pages", "download.tsx"), join("lib", "api.ts")]),
    );
    const found = files.flatMap((path) => offenders(readFileSync(path, "utf8"), relative(SRC, path)));

    expect(found, "build the URL from API_BASE (@/lib/api); see pages/download.tsx").toEqual([]);
  });

  // The scan above passing proves nothing unless the rule rejects the shapes it exists for.
  it("rejects the shapes that resolve against the web origin", () => {
    const bad = [
      'const downloadHref = (p: string) => `/api/v1/public/desktop/download/${p}`;',
      '<a href="/api/v1/public/desktop/download/macos">macOS</a>',
      "<a href={`/api/v1/x/${id}`}>x</a>",
      'window.location.href = "/api/v1/x";',
      'window.open("/api/v1/x");',
      'location.assign("/api/v1/x");',
    ];
    for (const line of bad) expect(offenders(line, "x.tsx"), line).toEqual(["x.tsx:1"]);
  });

  // A `/*` inside a line comment or a string is not a block comment. Read as one, it hid everything up
  // to the next `*/` from the scan: `lib/historical.ts`'s `/api/v1/stats/hist/*` comment, and
  // `lib/aircraft-icons.ts`'s glob string, each blanked real code.
  it("still scans code after a `/*` inside a line comment or a string", () => {
    const hidden = [
      '// the `/api/v1/stats/hist/*` endpoint\nexport const leak = "/api/v1/x";\n/** doc */',
      'const glob = "../assets/aircraft/*.svg";\n<a href="/api/v1/x">x</a>;\n/** doc */',
      "const glob = `../assets/*.svg`;\nlocation.href = '/api/v1/x';\n/** doc */",
    ];
    for (const src of hidden) expect(offenders(src, "x.tsx"), src).toEqual(["x.tsx:2"]);
  });

  it("accepts typed-client calls, a based URL, casts and comments", () => {
    const good = [
      'await ois.GET("/api/v1/me");',
      'await ois.POST(\n  "/api/v1/events/{id}/discord/publish",\n  {},\n);',
      'await call("/api/v1/admin/groups/{name}/members", {});',
      'await ois.POST(\n  `/api/v1/tmu/tmis/{id}/${verb}` as "/api/v1/tmu/tmis/{id}/publish",\n);',
      'new URL("/api/v1/ws", base);',
      "// links to `/api/v1/public/desktop/download/{platform}`",
      "/** see `/api/v1/stats/hist/*` */",
      "const u = `${API_BASE}/api/v1/public/desktop/download/macos`;",
      'const {data} = await ois.GET(path); // proxies "/api/v1/me"',
      'const docs = "https://example.org/a//b"; // not a comment start inside the string',
    ];
    for (const src of good) expect(offenders(src, "x.tsx"), src).toEqual([]);
  });
});
