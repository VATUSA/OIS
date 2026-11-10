import {readFileSync} from "node:fs";
import path from "node:path";

import {describe, expect, it} from "vitest";

import {navVersionText, parseStage, stageLabel, stageTitle, stageTone, stagedTitle} from "./stage";

// Node environment: `stage.ts` is DOM-free because `web/vite.config.ts` and
// `docs-site/.vitepress/config.mts` import it. Expected strings are literals from the issue and the
// approved plan (#841), never built from `stageLabel`, which would move both sides together.

describe("parseStage (#841)", () => {
  for (const value of ["alpha", "beta", "rc", "stable"] as const) {
    it(`parses "${value}" to that stage`, () => {
      expect(parseStage(value)).toBe(value);
    });
  }

  it("trims a trailing newline and surrounding spaces before matching", () => {
    expect(parseStage("alpha\n")).toBe("alpha");
    expect(parseStage("  rc \r\n")).toBe("rc");
  });

  it("throws on an empty string", () => {
    expect(() => parseStage("")).toThrow(/STAGE must be one of/);
  });

  it("throws on a whitespace-only string", () => {
    expect(() => parseStage(" \n\t")).toThrow(/STAGE must be one of/);
  });

  it("throws on an unknown value and names every allowed value in the message", () => {
    expect(() => parseStage("gamma")).toThrow('STAGE must be one of alpha, beta, rc, stable; got "gamma"');
  });

  it('throws on "Alpha" and "ALPHA": only lowercase values are accepted', () => {
    expect(() => parseStage("Alpha")).toThrow(/STAGE must be one of/);
    expect(() => parseStage("ALPHA")).toThrow(/STAGE must be one of/);
  });

  it('throws on "release-candidate", "ga" and "alpha beta"', () => {
    for (const value of ["release-candidate", "ga", "alpha beta"]) {
      expect(() => parseStage(value), value).toThrow(/STAGE must be one of/);
    }
  });

  it("parses the repo's root STAGE file (the value itself is not pinned: changing it needs no code edit)", () => {
    const raw = readFileSync(path.resolve(import.meta.dirname, "../../../STAGE"), "utf8");
    expect(["alpha", "beta", "rc", "stable"]).toContain(parseStage(raw));
  });
});

describe("stageLabel, stageTone and stageTitle (#841)", () => {
  it('gives "Alpha", "Beta" and "RC" for alpha, beta and rc, and "" for stable', () => {
    expect(stageLabel("alpha")).toBe("Alpha");
    expect(stageLabel("beta")).toBe("Beta");
    expect(stageLabel("rc")).toBe("RC");
    expect(stageLabel("stable")).toBe("");
  });

  it("gives the warn tone for alpha, brand for beta and neutral for rc", () => {
    expect(stageTone("alpha")).toBe("warn");
    expect(stageTone("beta")).toBe("brand");
    expect(stageTone("rc")).toBe("neutral");
  });

  it("gives the hover text for alpha, beta and rc", () => {
    expect(stageTitle("alpha")).toBe("Alpha: expect rough edges and frequent changes");
    expect(stageTitle("beta")).toBe("Beta: early access, still stabilizing");
    expect(stageTitle("rc")).toBe("Release candidate");
  });
});

describe("stagedTitle (#841)", () => {
  it('turns "OIS" into "OIS (Alpha)" for alpha', () => {
    expect(stagedTitle("OIS", "alpha")).toBe("OIS (Alpha)");
  });

  it('turns "OIS · TMU board" into "OIS (Alpha) · TMU board" for alpha', () => {
    expect(stagedTitle("OIS · TMU board", "alpha")).toBe("OIS (Alpha) · TMU board");
  });

  it('turns the panel title "ZDC · metering" into "ZDC · metering (Alpha)" for alpha', () => {
    expect(stagedTitle("ZDC · metering", "alpha")).toBe("ZDC · metering (Alpha)");
  });

  it('uses "Beta" and "RC" in the title for beta and rc', () => {
    expect(stagedTitle("OIS", "beta")).toBe("OIS (Beta)");
    expect(stagedTitle("OIS · TMU board", "rc")).toBe("OIS (RC) · TMU board");
  });

  it('returns "OIS", "OIS · TMU board" and "ZDC · metering" unchanged for stable', () => {
    for (const title of ["OIS", "OIS · TMU board", "ZDC · metering"]) {
      expect(stagedTitle(title, "stable")).toBe(title);
    }
  });

  it('treats "OISX" and "OIS·x" as panel titles, not as the app name', () => {
    expect(stagedTitle("OISX", "alpha")).toBe("OISX (Alpha)");
    expect(stagedTitle("OIS·x", "alpha")).toBe("OIS·x (Alpha)");
  });
});

describe("navVersionText (#841)", () => {
  it('gives "v1.0.1-abc1234 · Alpha" for alpha', () => {
    expect(navVersionText("1.0.1-abc1234", "alpha")).toBe("v1.0.1-abc1234 · Alpha");
  });

  it('gives "v1.0.1-abc1234 · RC" for rc', () => {
    expect(navVersionText("1.0.1-abc1234", "rc")).toBe("v1.0.1-abc1234 · RC");
  });

  it('gives "v1.0.1-abc1234" with nothing appended for stable', () => {
    expect(navVersionText("1.0.1-abc1234", "stable")).toBe("v1.0.1-abc1234");
  });
});
