import {describe, expect, it} from "vitest";

import {widgetTitle} from "./render";
import type {AtcWidget} from "./types";

const atc = (facility: AtcWidget["facility"]): AtcWidget => ({ id: "w", kind: "atc", facility });

describe("widgetTitle for an ATC widget (VATUSA/OIS#474)", () => {
  it("names the NAS when the widget is scoped nationally", () => {
    expect(widgetTitle(atc({ kind: "national" }))).toBe("NAS · ATC");
  });

  it("still names the facility otherwise", () => {
    expect(widgetTitle(atc({ kind: "artcc", id: "ZDC" }))).toBe("ZDC · ATC");
    expect(widgetTitle(atc({ kind: "tracon", id: "PCT" }))).toBe("PCT · ATC");
  });

  it("lets an explicit title win over both", () => {
    expect(widgetTitle({ ...atc({ kind: "national" }), title: "Who's on" })).toBe("Who's on");
  });
});
