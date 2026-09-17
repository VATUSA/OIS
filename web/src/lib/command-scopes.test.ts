import {describe, expect, it} from "vitest";

import {SCOPES, parseScopePrefix} from "./command-scopes";

describe("parseScopePrefix", () => {
  it("switches to a uniquely-prefixed scope and strips the prefix", () => {
    expect(parseScopePrefix("@tmi ZNY", SCOPES)).toEqual({ scope: "tmis", rest: "ZNY" });
    expect(parseScopePrefix("@AIRC ", SCOPES)).toEqual({ scope: "aircraft", rest: "" });
    expect(parseScopePrefix("@airport kden", SCOPES)).toEqual({ scope: "airports", rest: "kden" });
  });

  it("needs the trailing space, so a half-typed prefix stays in the query", () => {
    expect(parseScopePrefix("@tmi", SCOPES)).toBeNull();
  });

  it("ignores an ambiguous prefix", () => {
    // Aircraft and Airport data both start with "air".
    expect(parseScopePrefix("@air ", SCOPES)).toBeNull();
  });

  it("only matches scopes the user can see", () => {
    const noTmis = SCOPES.filter((s) => s.id !== "tmis");
    expect(parseScopePrefix("@tmi ", noTmis)).toBeNull();
  });

  it("leaves plain queries alone", () => {
    expect(parseScopePrefix("DAL123", SCOPES)).toBeNull();
    expect(parseScopePrefix("@ nothing", SCOPES)).toBeNull();
  });
});
