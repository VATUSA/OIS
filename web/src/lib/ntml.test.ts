import {readFileSync} from "node:fs";

import {describe, expect, it} from "vitest";

import {decodeNtml, encodeNtml, type Ntml} from "./ntml";

/** One reference case from the shared fixture. Mirrors `Case` in `backend/src/tmi.rs`'s tests. */
interface Case {
  name: string;
  why: string;
  restriction: Ntml;
  raw: string;
  english: string;
}

// Read rather than `import`: the fixture lives at the repo root, outside this package, which a
// static JSON import would have to be allowed through Vite's `server.fs` list to reach. The test
// runs in node, so reading it is both simpler and exactly what the Rust side does.
const FIXTURE_URL = new URL("../../../fixtures/ntml-reference.json", import.meta.url);
const {cases} = JSON.parse(readFileSync(FIXTURE_URL, "utf8")) as {cases: Case[]};

// VATUSA/OIS#455 — this grammar is implemented twice, here and in `backend/src/tmi.rs`. Both read
// this one file, so a case is written once and the two cannot drift apart without one side going
// red. Before the fixture was shared they had already drifted: the backend accepted `<=`/`>=` as
// speed operators and this mirror did not, so a TMI carrying one was previewed in the form as
// "at 250kt" and published to the channel as "at or below 250kt".
describe("NTML codec against the shared fixtures (VATUSA/OIS#455)", () => {
  it("has cases to check", () => {
    // Without this, a fixture that failed to load or was emptied would make every assertion below
    // vacuous and the suite would stay green while testing nothing.
    expect(cases.length).toBeGreaterThan(0);
  });

  it.each(cases.map((c) => [c.name, c] as const))("encodes %s to the shared raw line", (_name, c) => {
    expect(encodeNtml(c.restriction)).toBe(c.raw);
  });

  it.each(cases.map((c) => [c.name, c] as const))("decodes %s to the shared English", (_name, c) => {
    expect(decodeNtml(c.restriction)).toBe(c.english);
  });
});
