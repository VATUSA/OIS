import {readdir, readFile} from "node:fs/promises";
import {resolve} from "node:path";

import {beforeAll, describe, expect, it} from "vitest";

/**
 * Nobody may call a Tauri unlisten handle bare (#426).
 *
 * `safeUnlisten` has thorough tests of its own, but they prove only that the helper works — reverting
 * every call site to `dispose?.()` left the whole suite green, so the bug could walk straight back in
 * (#426 review). A per-component test would only cover the components someone thought to write one
 * for, and the way this issue arose was a site nobody was looking at.
 *
 * So the guard is a source scan. It fails on a *new* unprotected site as readily as on a reverted one
 * — including the one a merge conflict silently drops, which is exactly how #419's rewrite of
 * `window-controls.tsx` would otherwise have taken the fix back out.
 *
 * It is deliberately crude: it looks for a call of a variable whose name says it is a listener
 * teardown. A handle under some other name slips through, and that is the accepted limit — the point
 * is to catch the idiom the codebase actually uses, not to prove a negative.
 */
const SRC = resolve(process.cwd(), "src");

/** The names this codebase gives an unlisten handle. */
const HANDLE = /\b(unlisten|dispose|off[A-Z]\w*|offDesktop|stop|d)\b/;

/**
 * `foo();` or `foo?.();` alone on a line — not `safeUnlisten(foo)`, which is the point.
 *
 * Anchored to the line rather than to the previous `;`, because a consuming prefix eats the semicolon
 * that would start the next match and every second offender goes unseen.
 */
const BARE_CALL = /^[ \t]*(?:await\s+)?([A-Za-z_$][\w$]*)\s*(?:\?\.)?\(\s*\)\s*;/gm;

/**
 * ...and nobody may throw a handle away unregistered (#439).
 *
 * The scan above catches a handle that is *called* bare. It cannot catch one that was never kept:
 * `await win.onMoved(remember);` registers a listener and drops the function that would remove it,
 * so there is no teardown call for `BARE_CALL` to find — which is exactly why `popout.ts` leaked a
 * pair of geometry listeners per window opened, for the life of the session, with this file already
 * green. Two different defects, so two different scans.
 *
 * Matches an awaited registration whose result goes nowhere: the statement *begins* with `await`,
 * so `const off = await win.onMoved(…)` is fine and `await win.onMoved(…)` is not. Covers both a
 * method (`win.onMoved`, `current.onCloseRequested`) and the bare `listen(…)` import.
 *
 * `once(…)` is deliberately absent: it removes itself after firing, so its handle is not worth
 * keeping and `popout.ts` discards one on purpose.
 */
const DISCARDED = /^[ \t]*await\s+(?:[A-Za-z_$][\w$]*\.)*(on[A-Z]\w*|listen)\s*\(/gm;

/**
 * Files that legitimately call a handle bare. `desktop-events.ts` is the helper itself; the tests
 * construct handles on purpose.
 */
const ALLOWED = /(?:desktop-events\.ts|\.test\.tsx?$)/;

async function sources(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir, {withFileTypes: true})) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await sources(path)));
    else if (/\.tsx?$/.test(entry.name)) out.push(path);
  }
  return out;
}

let offenders: string[] = [];
let discarded: string[] = [];
beforeAll(async () => {
  const files = (await sources(SRC)).filter((f) => !ALLOWED.test(f));
  offenders = [];
  discarded = [];
  for (const file of files) {
    const text = await readFile(file, "utf8");
    // Comments would otherwise match the prose that explains this very rule.
    const code = text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
    const lines = code.split("\n");
    const at = (index: number) => {
      const line = code.slice(0, index).split("\n").length;
      return `${file.slice(SRC.length + 1)}:${line}  ${lines[line - 1]?.trim()}`;
    };
    for (const match of code.matchAll(BARE_CALL)) {
      const name = match[1];
      if (!HANDLE.test(name)) continue;
      offenders.push(at(match.index));
    }
    for (const match of code.matchAll(DISCARDED)) discarded.push(at(match.index));
  }
});

describe("unlisten handles", () => {
  it("are never called bare — every teardown goes through safeUnlisten", () => {
    expect(offenders).toEqual([]);
  });

  /** The scanner has to actually see one, or the test above is decoration. */
  it("the scanner recognises a bare call and ignores a guarded one", () => {
    const bare = [...`  dispose?.();\n  unlisten();\n`.matchAll(BARE_CALL)].map((m) => m[1]);
    expect(bare.filter((n) => HANDLE.test(n))).toEqual(["dispose", "unlisten"]);

    const guarded = [...`  safeUnlisten(dispose);\n`.matchAll(BARE_CALL)].map((m) => m[1]);
    expect(guarded.filter((n) => HANDLE.test(n))).toEqual([]);
  });
});

describe("unlisten handles, continued", () => {
  it("are never discarded — every registration keeps what it can unregister with", () => {
    expect(discarded).toEqual([]);
  });

  /** Same reasoning as above: a scanner that matches nothing passes silently. */
  it("the scanner recognises a discarded registration and ignores a kept one", () => {
    const thrownAway = [
      "  await win.onMoved(remember);",
      "  await current.onCloseRequested(() => {});",
      "  await listen(SUSPEND_EVENT, () => {});",
    ].join("\n");
    expect([...thrownAway.matchAll(DISCARDED)].map((m) => m[1])).toEqual([
      "onMoved",
      "onCloseRequested",
      "listen",
    ]);

    const kept = [
      "  const offMoved = await win.onMoved(remember);",
      "  offClose = await current.onCloseRequested(() => {});",
      "  const off = await listen(SUSPEND_EVENT, () => {});",
      // `once` removes itself, so discarding its handle is correct.
      '  void win.once("tauri://destroyed", () => {});',
    ].join("\n");
    expect([...kept.matchAll(DISCARDED)]).toEqual([]);
  });
});
