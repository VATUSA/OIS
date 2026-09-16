import {describe, expect, it} from "vitest";

import {auditActionTone, jobState, toneOf} from "./status";

describe("toneOf", () => {
  it("maps known statuses and falls back to neutral", () => {
    expect(toneOf("publish", "published")).toBe("good");
    expect(toneOf("publish", "cancelled")).toBe("bad");
    expect(toneOf("flight", "airborne")).toBe("airborne");
    expect(toneOf("category", "IFR")).toBe("ifr");
    expect(toneOf("publish", "mystery")).toBe("neutral");
    expect(toneOf("dcc", null)).toBe("neutral");
  });
});

describe("auditActionTone", () => {
  it("reads the verb", () => {
    expect(auditActionTone("user.role.grant")).toBe("good");
    expect(auditActionTone("api_key.revoke")).toBe("bad");
    expect(auditActionTone("settings.update")).toBe("neutral");
  });
});

describe("jobState", () => {
  it("prefers running, then never-run, then last outcome", () => {
    expect(jobState({ running: true, last_ok: false })).toEqual({ tone: "brand", label: "Running…" });
    expect(jobState({ running: false, last_ok: null })).toEqual({ tone: "neutral", label: "Never run" });
    expect(jobState({ running: false, last_ok: false })).toEqual({ tone: "bad", label: "Failed" });
  });
});
