import {readdir, readFile} from "node:fs/promises";
import {resolve} from "node:path";

import {beforeAll, describe, expect, it} from "vitest";

/**
 * The desktop window keeps each platform's own title bar (#796), so no source in the app may draw
 * window chrome of its own: no traffic-light replica, no minimize / maximize buttons, no Tauri drag
 * region. #402 and #419 had all three; this keeps them from coming back.
 *
 * A source scan, not a render test, on purpose. `app-sidebar.window-chrome.test.tsx` renders the one
 * place the replica used to live, but a replica could return anywhere: the signed-out landing page and
 * the error screens carried their own copy (#423), and `router.tsx` imports every page in the app and
 * cannot be mounted in jsdom. Scanning every web and `@ois/ui` source catches a new site as readily
 * as a reverted one. Don't swap it for something narrower.
 *
 * Comments are stripped first, because prose that explains the old chrome is not chrome. The
 * stripping does not know about strings: a `/*` inside a string literal (a glob such as
 * `"../assets/*.svg"`) hides the code up to the next `*\/`, and a `//` that is not part of `://`
 * hides the rest of its line. That is the accepted limit.
 */
const ROOTS = [resolve(process.cwd(), "src"), resolve(process.cwd(), "../packages/ui/src")];

/** What app-drawn window chrome looks like in source, by every name it went by. */
const FORBIDDEN: readonly [name: string, pattern: RegExp][] = [
  ["a Tauri drag region", /data-tauri-drag-region/],
  ["a call that drags the window", /\bstartDragging\s*\(/],
  ["an app-drawn minimize / maximize button", /\.(?:minimize|maximize|toggleMaximize)\s*\(/],
  // Selectors, class names and tokens, not the words: release notes may still say "traffic light".
  ["the traffic-light replica", /\.traffic-lights?\b|(?:className|\bcn\(|\bclsx\()[^\n]*\btraffic-lights?\b|--traffic-/],
];

function strip(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

/** `file:line  what` for every forbidden pattern in `code`. */
function scan(file: string, text: string): string[] {
  const code = strip(text);
  const hits: string[] = [];
  for (const [name, pattern] of FORBIDDEN) {
    const global = new RegExp(pattern.source, "g");
    for (const match of code.matchAll(global)) {
      const line = code.slice(0, match.index).split("\n").length;
      hits.push(`${file}:${line}  ${name}`);
    }
  }
  return hits;
}

async function sources(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir, {withFileTypes: true})) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await sources(path)));
    else if (/\.(tsx?|css)$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) out.push(path);
  }
  return out;
}

let scanned = 0;
let offenders: string[] = [];
beforeAll(async () => {
  offenders = [];
  scanned = 0;
  for (const root of ROOTS) {
    for (const file of await sources(root)) {
      scanned++;
      offenders.push(...scan(file.slice(root.length - root.split("/").pop()!.length), await readFile(file, "utf8")));
    }
  }
});

describe("app-drawn window chrome (#796)", () => {
  it("is drawn nowhere in the web app or @ois/ui — the OS draws the title bar", () => {
    // Positive control: a scan that found no files would pass with nothing scanned.
    expect(scanned).toBeGreaterThan(100);
    expect(offenders).toEqual([]);
  });

  /** The scanner has to see each shape the old chrome took, or the test above is decoration. */
  it("the scanner recognises the #402/#419 chrome and ignores prose about it", () => {
    const old = [
      `<div data-tauri-drag-region className="h-11" />`,
      `await win.startDragging();`,
      `onClick={act((win) => win.minimize())}`,
      `onClick={act((win) => win.toggleMaximize())}`,
      `<div className="flex traffic-lights">`,
      `  background: var(--traffic-close);`,
      `<div className={cn("traffic-lights", blurred && "dim")}>`,
      "<button className={`traffic-light ${tone}`} />",
    ].join("\n");
    expect(scan("old.tsx", old)).toEqual([
      "old.tsx:1  a Tauri drag region",
      "old.tsx:2  a call that drags the window",
      "old.tsx:3  an app-drawn minimize / maximize button",
      "old.tsx:4  an app-drawn minimize / maximize button",
      "old.tsx:5  the traffic-light replica",
      "old.tsx:6  the traffic-light replica",
      "old.tsx:7  the traffic-light replica",
      "old.tsx:8  the traffic-light replica",
    ]);

    const fine = [
      `// The window no longer has a data-tauri-drag-region or traffic-lights (#796).`,
      `/* win.toggleMaximize() was the replica's */`,
      `await existing.unminimize().catch(() => undefined);`,
      `const url = "https://example.com/a";`,
      `const note = "No more traffic-light replica on Windows";`,
    ].join("\n");
    expect(scan("fine.tsx", fine)).toEqual([]);
  });
});
